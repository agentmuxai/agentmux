# Performance instrumentation + optimization strategy

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-09
**Driving observation:** Tab switching and pane resizing both have visibly long delays. AgentMux's brand promise is **ultra-snappy responsiveness**; today we have neither numerical baselines nor a structured way to find bottlenecks.

## Goals

1. **Numerical targets** — every user-initiated interaction lands inside the [Web Vitals "Good INP" budget](https://web.dev/articles/inp): **P75 INP ≤ 200 ms, P95 ≤ 500 ms**. Internal "snappy" target: **P50 ≤ 100 ms, P95 ≤ 200 ms** for tab switch and pane resize specifically.
2. **Repeatable measurement** — every PR that touches a hot path captures a before/after profile. No "felt faster" merges.
3. **Always-on bottleneck visibility** — a perf HUD in dev mode shows recent INP, frame budget overrun count, and IPC roundtrip distribution.
4. **No regression discipline** — once we hit a target, capture a baseline trace that future PRs are measured against.

Snappy-by-default is a structural property — it has to be designed into the dispatch path, not added at the end. The output of this work is an instrumented runtime, not "an optimization PR". Every fix lands as evidence-driven incremental change.

## Scope (in priority order)

### Tier 1 — user-initiated interactions (tracked from day one)

| Interaction | Initiator | Hypothesized hot paths |
|---|---|---|
| **Tab switch** | click on tab in title bar | `activeTabId` atom update → all per-tab Solid effects re-evaluate; per-block visibility toggling; pane HWND show/hide IPC fan-out (browser, terminal); focus shuffle |
| **Pane resize (drag)** | mouse drag on splitter | continuous `getBoundingClientRect` (forces layout) per frame; `ResizeObserver` callback fires; `browser_pane_resize` IPC roundtrip per pane per frame; Win32 `SetWindowPos` per pane HWND |
| **Block focus** | click into a pane | `refocusNode(blockId)` → layout reducer dispatch → `giveFocus()` → IPC; the focus path we just instrumented for slice #9's pane-click bug |

### Tier 2 — secondary interactions (instrument once Tier 1 is solved)

| Interaction | Notes |
|---|---|
| Agent message send (`agent.send`) | Streaming output paint cost; reactive fan-out per chunk |
| Terminal keystroke | xterm.js render, per-key IPC for shell stdin |
| Pane open/close | Block create/dispose; HWND attach/detach |
| Window create | New OS window + main webview spin-up |

### Out of scope (first pass)

- GPU compositor frame timing (`gpu/RenderCompositorFrame`) — kick in once we've eliminated CPU-bound bottlenecks.
- Memory pressure / GC pauses — separate investigation; only chase if traces show GC events ≥10 ms during interactions.
- Sidecar query latency — `agentmux-srv` isn't on the synchronous interaction path for tab switch / pane resize. Defer.

## Measurement methodology

Every interaction is a **trace span** with a known start and end. Three complementary tools cover the JS-renderer / Win32 / Rust-host axes.

### A. Renderer-side (main webview React + each pane CEF)

#### A1. `performance.mark()` / `performance.measure()` — primary signal

Wrap every Tier 1 interaction with named marks. Example for tab switch:

```ts
// At click handler entry
performance.mark("tab-switch:start", { detail: { from: prev, to: next } });
// At end of layout commit (createEffect end of cycle)
performance.mark("tab-switch:committed");
performance.measure("tab-switch:total", "tab-switch:start", "tab-switch:committed");
```

The measures are visible in the Performance Panel timeline (under "Timings") and queryable via `PerformanceObserver`.

#### A2. Long Tasks API observer — always-on alarm

Subscribe at app startup to log any task >50 ms:

```ts
new PerformanceObserver((list) => {
    for (const entry of list.getEntries()) {
        if (entry.duration > 50) {
            console.warn(`[perf] long-task ${entry.duration.toFixed(1)}ms`, entry);
        }
    }
}).observe({ type: "longtask", buffered: true });
```

Long tasks are the single best proxy for "frame budget violations". One alarm per frame > 50 ms. Logs route through the `[fe]` pipe to host so they show in `muxlog host '\[perf\]'`.

#### A3. INP-specific observer

The browser exposes `event` + `first-input` entry types via `PerformanceObserver`. Subscribe and track P50/P75/P95 INP per interaction-target (button name, atom name, blockId). Expose in dev-mode HUD.

#### A4. Chromium tracing via CDP — heavyweight on-demand

The host already has CDP access to every CEF browser ([`agentmux-cef/src/browser_api/`](../../agentmux-cef/src/browser_api/)). Add an endpoint pair to start/stop a [Chromium trace](https://www.chromium.org/developers/how-tos/trace-event-profiling-tool/) and write a `.json.gz` to disk:

```
POST /agentmux/perf/trace_start { window_label?, categories: ["devtools.timeline","blink","cc","gpu","v8.execute"] }
POST /agentmux/perf/trace_stop  → returns path to trace file
```

Output drops in [Perfetto UI](https://ui.perfetto.dev) for analysis. This is the single most powerful tool — gives compositor frames, paint timing, layout cost, JS sample profile, IPC routing, all on one timeline.

Categories table:

| Category | Use for |
|---|---|
| `devtools.timeline` | high-level events shown in DevTools Performance panel |
| `blink` | layout, paint, style invalidations |
| `cc` | compositor/raster/GPU |
| `v8.execute` | JS sample profile |
| `disabled-by-default-ipc.flow` | IPC routing across processes (heavy — only for IPC investigations) |
| `disabled-by-default-cpu_profiler` | sample-based CPU profile |

#### A5. SolidJS effect-run counter (dev-only)

Add a `wrapEffect()` helper that wraps `createEffect` and increments a per-effect run counter. Expose the counter table via the dev HUD. Identifies effect-storm patterns ([feedback_solidjs_reactive_leak](../../../../../../Users/area54/.claude/projects/C--Systems/memory/feedback_solidjs_reactive_leak.md) bit us before; this catches it pre-flight).

### B. Host-side (Rust — agentmux-cef + agentmux-srv)

#### B1. `tracing` spans at IPC boundaries — already partially in place

We already use `tracing` everywhere; what's missing is consistent instrumentation at IPC entry/exit. Add `#[tracing::instrument(level = "debug")]` to every JSON-RPC handler in `agentmux-cef/src/commands/` and to `agentmux-srv` RPC handlers. Span enter/exit timestamps give per-IPC roundtrip latency.

#### B2. Chrome-trace-format export from Rust

[`tracing-chrome`](https://crates.io/crates/tracing-chrome) writes `tracing` spans into a `.json` consumable by [Perfetto UI](https://ui.perfetto.dev). Combine with renderer-side trace (A4) — load both into the same Perfetto session, see the full picture across processes.

Gate behind `--features perf-trace` so release builds don't pay the cost. Dev mode opt-in.

#### B3. Win32 message pump timing (lighter than ETW)

For pane resize the bottleneck is likely `SetWindowPos` calls in series. Instrument `pane/hwnd.rs::resize()` with `tracing::trace!` spans; the existing `[pane-wndproc] key msg=` infrastructure shows the pattern. Trace pump turnaround time per `SetWindowPos`.

### C. Cross-process (renderer ↔ host)

#### C1. IPC roundtrip clock

Today `invokeCommand` (CEF JS bridge) doesn't time its own roundtrip. Add bookend marks:

```ts
// frontend/app/platform/ipc.ts
async function invokeCommand(name, args) {
    const t0 = performance.now();
    const result = await rawInvoke(name, args);
    const dt = performance.now() - t0;
    if (dt > 16) console.warn(`[perf] ipc ${name} ${dt.toFixed(1)}ms`, args);
    return result;
}
```

16 ms = one frame at 60 Hz. Anything over is a frame-budget threat. This single change reveals the chatty-IPC class of issues that bit Slack and VSCode (per Electron community write-ups — same applies here even though we're CEF, not Electron).

## Hypotheses going in (to be confirmed by traces)

These guide instrumentation priority but **the traces are authoritative** — don't optimize before measuring.

### H1. Pane resize is dominated by per-frame IPC

`browser-view.tsx:syncPosition()` fires on every `ResizeObserver` callback and calls `browser_pane_resize`. With N panes and 60 Hz drag, that's 60×N IPC roundtrips per second per drag. Each roundtrip is at least one main-thread message hop on each side; even 1 ms × 60 × N = 60 ms × N spent on IPC alone per second. **Likely fix candidates** (decide after traces): debounce/coalesce per frame, batch all pane resizes into a single IPC, switch to a non-IPC geometry sync via shared state.

### H2. Tab switch has effect-storm fan-out

Switching `activeTabId` triggers per-tab visibility memos in every block. With N tabs × M blocks/tab, the dependency graph re-runs O(NM). Most of those are no-ops (visibility didn't change for that block), but the createEffect runs anyway. Measure with A5 (effect-run counter) and confirm.

### H3. Pane HWND show/hide is serialized through the host UI thread

Tab switch needs to hide all browser-pane HWNDs of the previous tab and show the next tab's. Today these go through `browser_pane_hide` / `browser_pane_show` IPCs one at a time. Each one needs a Win32 `SetWindowPos` with `SWP_HIDEWINDOW`/`SWP_SHOWWINDOW` plus a parent HWND lock. If serialized, that's N × pump-roundtrip latency. **Possible fix:** batch HWND visibility into one IPC handler that calls `BeginDeferWindowPos` / `DeferWindowPos` / `EndDeferWindowPos` to atomically reposition them all.

### H4. Long synchronous Solid memos in render path

Per [our own past incident](../../../../../../Users/area54/.claude/projects/C--Systems/memory/feedback_solidjs_reactive_leak.md), a single bad signal-getter in a shared utility can cascade through the reactive graph. Worth ruling out via A5 before assuming it's gone.

## Phased plan

### Phase 0 — Instrumentation infrastructure (no behavior change)

One PR. Adds A1, A2, A3, A5 plus a dev-mode perf HUD. Adds B1 to IPC handlers. Adds C1 to `invokeCommand`. Behind a `[perf]` log tag so existing recipes are unaffected.

**Acceptance:** running `task dev`, opening `muxlog host '\[perf\]'`, and reproducing a tab switch / pane resize shows clean `tab-switch:total=Xms` measure entries plus IPC roundtrip warnings if any are >16 ms.

**Effort:** ~300 LOC, 0.5 day.

### Phase 1 — Baseline measurement

No code change. Capture and document baselines for the Tier 1 interactions in [`docs/retro/perf-baseline-2026-05-09.md`](../retro/) (or whatever date this lands). For each interaction:

- 5+ trials, P50/P75/P95
- 1 representative Chromium trace + 1 `tracing-chrome` trace per interaction
- Long-tasks log
- Initial hypothesis correlation: do the traces match H1–H4?

**Effort:** 0.5 day with the harness from Phase 0.

### Phase 2 — Hypothesis-driven optimization (one fix per PR)

Each PR:

1. Cite the baseline trace + the bottleneck identified.
2. Implement the targeted fix.
3. Capture an after-trace under the same workload.
4. Prove the improvement quantitatively (P75 dropped from X to Y).
5. Ship as a single PR with the trace-pair attached as a `docs/retro/` entry.

**No "let me also fix this thing" combos.** One bottleneck per PR; bundling makes regression bisection painful and review complexity grows superlinearly.

**Effort:** ~1 day per fix. Estimate 3–5 fixes to hit the Tier 1 targets.

### Phase 3 — Continuous monitoring + regression guards

- Dev-mode perf HUD: floating panel showing recent INP, long-task count, IPC P95. Toggle with a keyboard shortcut. Hidden in release builds.
- Optional CI perf-regression test: programmatic harness (using the existing browser API + new `/agentmux/perf/*` endpoints) that captures traces for a fixed workload and fails if INP exceeds threshold. Ride this with the implementation when it lands; doc-only spec for now.

**Effort:** HUD ~0.5 day; regression CI deferred.

## Dev-mode perf HUD (sketch)

A small floating panel in the corner, toggled by `Ctrl+Shift+P`. Shows:

```
─── Perf ───
Recent INP:  P50 47ms  P75 89ms  P95 230ms
Long tasks:  3 in last 5s (max 187ms)
IPC P95:     12ms (browser_pane_resize)
Effects:     128 runs/s (top: BlockFrame.visibility 47/s)
```

Backed by `PerformanceObserver` aggregations. Refreshes every second. Click to expand to a full timeline of recent interactions. Hidden behind `is_dev` check; the existing `agentmux-cef` dev-mode plumbing covers this.

## Threat model / cost

The instrumentation itself adds cost. We must not let measurement BE the bottleneck.

- `performance.mark/measure`: ~50 ns each. Negligible at our call counts (<1000/s).
- Long Tasks API: zero cost — the browser already tracks tasks; we just observe.
- INP observer: zero cost (browser-internal).
- IPC roundtrip wrap: 1 perf.now() pair per call. Negligible.
- Chromium tracing (A4): **expensive** — slows the renderer 2–5×. Off by default; on-demand only.
- `tracing-chrome` (B2): minor (~1 µs/span). Off by default; gated behind `--features perf-trace`.

Phase 0 instrumentation should be net-zero in release (gated by `is_dev` checks where it isn't free).

## Out-of-scope / explicit non-goals

- **No WebPageTest / Lighthouse integration.** Those tools target public web pages; we're an embedded app with private auth. The Performance Panel + Perfetto UI are the right tools.
- **No bundle-size optimization** (this round). Code-splitting is a separate concern and the current frontend bundle is already past the immediate-startup horizon.
- **No premature React → preact / Solid → vanilla rewrite.** Slice-by-slice fixes against measured bottlenecks first. Architectural changes need traces to justify.

## Cross-references

- Web Vitals INP: [https://web.dev/articles/inp](https://web.dev/articles/inp)
- Chromium trace tool: [https://www.chromium.org/developers/how-tos/trace-event-profiling-tool/](https://www.chromium.org/developers/how-tos/trace-event-profiling-tool/)
- Perfetto UI: [https://ui.perfetto.dev](https://ui.perfetto.dev)
- Long Tasks API: [https://developer.mozilla.org/en-US/docs/Web/API/PerformanceLongTaskTiming](https://developer.mozilla.org/en-US/docs/Web/API/PerformanceLongTaskTiming)
- Electron perf community wisdom: [https://palette.dev/blog/improving-performance-of-electron-apps](https://palette.dev/blog/improving-performance-of-electron-apps)
- Past incident — Solid reactive leak: see memory `feedback_solidjs_reactive_leak.md`
- Reducer-stack work parallel: GitHub Discussion #707
