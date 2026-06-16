# SPEC: Status-bar CPU → per-core panel

**Date:** 2026-06-15
**Repo:** agentmuxai/agentmux @ v0.46.0
**Status:** Draft / ready to implement
**Effort:** ~Small–Medium. Frontend-only. ~1 new component with three adaptive render tiers (~200–240 LOC incl. the computed square-sizing + `loadColor` ramp) + ~15 LOC change to `SystemStats.tsx` + ~1 SCSS partial. **No backend changes.**

---

## 1. Goal

Clicking the **CPU** readout in the bottom status bar opens a small panel that lists **every logical CPU core** with its individual usage percentage (live-updating), plus the aggregate. Today the status bar shows only the system-wide average (`CPU 45%`) and the readout is not interactive.

---

## 2. Key finding — the data already exists

The backend **already collects and publishes per-core CPU** every sample tick; this feature is **purely frontend**.

`agentmux-srv/src/backend/sysinfo.rs:29-41` — `get_cpu_data()`:

```rust
// Total CPU usage (average across all cores)
let total: f64 = cpus.iter().map(|c| c.cpu_usage() as f64).sum::<f64>() / cpus.len() as f64;
values.insert("cpu".to_string(), total);
// Per-core usage
for (idx, cpu) in cpus.iter().enumerate() {
    values.insert(format!("cpu:{}", idx), cpu.cpu_usage() as f64);
}
```

- Event name: `EVENT_SYS_INFO` = `"sysinfo"` (`agentmux-srv/src/backend/wps.rs`).
- Scope: `"local"` (the connection label).
- Payload: `TimeSeriesData { ts: i64, values: HashMap<String, f64> }`.
- Cadence: `telemetry:interval` setting, default **1.0s** (range 0.2–2.0s), `sysinfo.rs:24`.
- `sysinfo` crate **0.34** (`agentmux-srv/Cargo.toml`).

**Runtime payload (`event.data.values`):**

```json
{ "cpu": 45.5, "cpu:0": 50.2, "cpu:1": 42.1, "cpu:2": 48.3, "cpu:3": 41.9,
  "gpu": 12.0, "mem:used": 8.5, "mem:total": 16.0, "...": 0 }
```

The panel extracts the aggregate `cpu` and every key matching `^cpu:\d+$`.

> Note: the existing sysinfo *graph* view already enumerates per-core keys (`frontend/app/view/sysinfo/sysinfo-types.ts` — `PlotTypes["All CPU"]` filters `cpu:*`≠`cpu`; `DefaultPlotMeta` pre-labels cores 0–31). We don't reuse the plot, but it confirms the key convention.

---

## 3. Current state (frontend)

| Concern | Location |
|---|---|
| Status bar container | `frontend/app/statusbar/StatusBar.tsx` (left section renders `BackendStatus → SystemStats → GpuStatus`) |
| CPU readout (target) | `frontend/app/statusbar/SystemStats.tsx:73-80` — static `<span class="stat-mono stat-cpu">CPU {n}%</span>` |
| sysinfo subscription | `SystemStats.tsx:48-66` — `waveEventSubscribe({ eventType: "sysinfo", scope: "local" })` |
| Color thresholds | `SystemStats.tsx:32-36` — `cpuColor()`: >95% error, >80% warning |
| **Popover pattern to mirror** | `TokenUsageIndicator.tsx` (button + open state + outside-click/Esc) → `TokenBreakdownPopover.tsx` (`usePaneOverlay`, `computeMenuPosition` placement `top-end`, `autoUpdate`, `assertMenuInPaintableArea`) |

The CPU panel should be a near-clone of the **TokenUsageIndicator → TokenBreakdownPopover** interaction, which is the canonical status-bar click-to-open-popover in this codebase.

---

## 4. Design

### 4.1 Interaction
- Convert the CPU `<span>` in `SystemStats.tsx` into a `<button class="stat-cpu-button">`.
- On click (and Enter/Space): capture `getBoundingClientRect()` as the anchor, toggle an `open` signal.
- Render `<CpuCoresPopover anchorRect onClose>` in a `<Portal>` while open.
- Close on: outside `mousedown` (ignoring the button + popover), Esc, or a second click on the button. (Identical to `TokenUsageIndicator.tsx:36-77`.)
- GPU / Mem / Net / Disk readouts stay non-interactive (unchanged).

### 4.2 Data flow
The popover subscribes to the **same** `sysinfo`/`local` event so cores update live while open (mirrors how `TokenBreakdownPopover` reads the live store). Each event:
- `aggregate = values["cpu"]`
- `cores = Object.keys(values).filter(k => /^cpu:\d+$/.test(k)).sort(byCoreIndex).map(k => ({ idx: Number(k.slice(4)), pct: values[k] }))`

Sort numerically by core index (string sort would order `cpu:10` before `cpu:2`).

A single shared subscription is acceptable: the button doesn't need core detail, only the popover does, so the popover owns its own `waveEventSubscribe` and tears it down on unmount. (Aggregate for the button keeps coming from `SystemStats`'s existing subscription — no change.)

### 4.3 Panel layout — adaptive density (scales from 4 to 256+ cores)

Core counts span a wide range: laptops report 4–16 logical CPUs; workstations/servers report **32, 64, 128**, and high-end boxes go higher. No single layout reads well across that whole range — a labeled bar per core is friendly at 8 and absurd at 128. So the panel is **adaptive**: it picks one of three representations from the core count, trading per-core label/detail for density as the count grows. The unit shrinks from a *row* → a *cell* → a *square*, but the metaphor (fill/color = load, sorted by index) stays consistent.

**Three tiers, chosen by `cores().length`:**

| Cores | Tier | Unit | Per-core detail shown inline | Detail on hover |
|---|---|---|---|---|
| ≤ 16 | **Rows** | full row | `Core 0  ▕███████░░░▏ 52%` — label + bar + % | — |
| 17–64 | **Cells** | compact tile | `C12` + `78%` + mini bar | — |
| 65+ | **Heatmap** | small square | color only (square filled by a load→color ramp) | tooltip `Core 37 — 64%` |

Thresholds (16, 64) are tunable constants. Rationale: keep the readable labeled rows for the common laptop/desktop case; switch to dense cells where 32/64-core machines still want the number visible; switch to a **heatmap of squares** where showing 128+ numbers at once is noise and a *spatial load picture* (which cores are hot) is what's actually useful.

**Tier 3 — heatmap squares (the "intelligent" high-count mode):**

```
CPU Usage                              avg 38%
128 cores                    ▕ idle ▓▓▓▓ busy ▏   ← color-ramp legend

■ ■ ■ ▣ ■ ■ ▣ ▣ ■ ■ ■ ■ ▣ ■ ■ ■    each ■ = one core, colored by load
■ ▣ ■ ■ ■ ■ ■ ▣ ■ ■ ▤ ■ ■ ■ ■ ■    hover a square → "Core 37 — 64%"
▣ ■ ■ ■ ▤ ■ ■ ■ ■ ■ ■ ■ ▣ ■ ■ ■
…
```

- **Square sizing is computed, not fixed** — the "intelligent" bit. Given the content width `W`, gap `g`, a target height `H`, and core count `N`, pick the **largest** square edge `s` in `[s_min=10px, s_max=22px]` at which all `N` cores fit within `H`, packing `cols = floor((W+g)/(s+g))` columns at that size:
  - Search `s` from `s_max` down; for each, `rows = ceil(N/cols)`; accept the first `s` where `rows*(s+g) − g ≤ H`.
  - Squares stay large while they fit (more legible) and **shrink only once `N` grows past what fits at the current size**; when even `s_min` overflows `H`, the grid keeps `s_min` and the area scrolls (height cap below). Net: ~64–128 cores stay near `s_max`, ~256 lands near `s_min`, beyond that scrolls.
- **Color ramp (continuous):** map `pct` → color across idle→busy, e.g. interpolate muted/blue (`var(--secondary-text-color)` / a cool stop) → `var(--warning-color)` → `var(--error-color)`. A continuous ramp (not the 3-step `cpuColor` threshold) is what makes a heatmap read; expose a small `loadColor(pct)` helper. Squares use the ramp as `background-color`; the existing 3-step `cpuColor` stays for the header/row-mode text.
- **Legend:** a tiny idle→busy gradient swatch in the header so the colors are interpretable.
- **Hover/focus:** each square is a focusable element with `title`/tooltip and `aria-label="Core N, X%"`; no persistent text inside the square. Optional enhancement: a single live "readout line" under the grid that shows the hovered/focused core's `Core N — X%` (cheaper than 128 tooltips, keyboard-friendly).

**Tier 2 — compact cells (17–64):** auto-fill grid, number stays visible.

```scss
.cpu-cores-grid {                                   /* tier 2 */
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(72px, 1fr));
  gap: 4px;
}
.cpu-cores-heatmap {                                /* tier 3 */
  display: grid;
  grid-template-columns: repeat(var(--cols), 1fr);  /* --cols set inline from computed c */
  gap: 2px;
}
```

**Common to all tiers:**
- **Header:** `CPU Usage` + aggregate `avg {Math.round(aggregate)}%` (colored via `cpuColor()`), legend in tier 3.
- **Subtitle:** `{n} cores`.
- **Sort:** numerically by core index.
- **Width (reactive by tier):** rows ~260px; cells ~360px; heatmap ~340–380px (enough for ≥8 squares/row).
- **Height cap:** content area `max-height: min(60vh, 420px); overflow-y: auto;`, header/subtitle/legend pinned above it. Tiers are sized to avoid scroll up to ~128 cores; beyond that it scrolls gracefully.
- **Render cost:** `<For>` keyed by core index (stable); per tick only each unit's fill/color/% updates. At 128 squares this is 128 cheap style updates/sec — fine. Virtualization only considered at 512+ (out of scope).

### 4.4 Positioning & airspace
Reuse exactly what `TokenBreakdownPopover` uses:
- `usePaneOverlay(() => rootRef)` so the popover paints over any browser-pane HWND it overlaps.
- `computeMenuPosition({ anchor, placement: "top-end" }, el)` + `@floating-ui/dom` `autoUpdate` (opens upward from the bottom bar, flips/shifts within the paintable area).
- `assertMenuInPaintableArea(el, "cpu-cores-popover")` dev guard.
- `data-pane-overlay` attribute on the root.

### 4.5 Accessibility
- Button: `aria-label="CPU usage, click for per-core breakdown"`, `data-tip="Per-core CPU usage"`, keyboard `Enter`/`Space` toggle.
- Popover root: `role="dialog"`, `aria-label="Per-core CPU usage"`.
- **Rows/cells (tiers 1–2):** each labeled; the bar is decorative (`aria-hidden`), the `%` text carries the value.
- **Heatmap squares (tier 3):** color alone is never the only signal — every square has `aria-label="Core N, X%"` (and a `title` for pointer hover), so the value is available to screen readers and the colorblind. The squares form a focusable group (roving `tabindex` or a `role="list"` of `role="listitem"`s); arrow-key navigation moves focus and drives the optional live readout line. The header keeps the exact aggregate number, so the panel is still usable with color perception off.

---

## 5. Files

**New**
- `frontend/app/statusbar/CpuCoresPopover.tsx` — the panel (clone `TokenBreakdownPopover.tsx` structure: `usePaneOverlay`, `registerFloating`/`computeMenuPosition`/`autoUpdate`, `role="dialog"`). Owns its own `waveEventSubscribe("sysinfo","local")`. Contains: the tier selector (rows/cells/heatmap by core count), the computed square-sizing for tier 3, a `loadColor(pct)` continuous ramp helper (alongside reusing the 3-step `cpuColor` for header text), and the optional hovered/focused-core readout line.
- `frontend/app/statusbar/_cpu-cores-popover.scss` — styles for all three tiers (rows, `.cpu-cores-grid`, `.cpu-cores-heatmap` with `--cols`/`--sq` custom props, legend swatch). Import into the status-bar SCSS aggregator alongside `_token-usage.scss`/`_instance-panel.scss`.

**Modified**
- `frontend/app/statusbar/SystemStats.tsx` — wrap the CPU readout in a button; add `open`/`anchorRect` signals + toggle/outside-click/Esc handlers (copy from `TokenUsageIndicator.tsx`); render `<CpuCoresPopover>` in a `<Portal>` when open. Export `cpuColor` (or move it to a shared util) for reuse in the popover.

**No changes**
- Backend (`sysinfo.rs`, `wps.rs`) — per-core data already published.
- No new RPC, no new event, no `rpc-api.ts` change.

---

## 6. Edge cases
- **No data yet** (panel opened before first sample): show `No CPU data yet.` empty state (rows length 0). The button itself only mounts once `SystemStats` has stats.
- **Core count changes** (rare; e.g. VM hot-plug): keys are recomputed each event, so the row set follows the latest payload.
- **Very high counts (32/64/128 cores):** first-class — the responsive grid (§4.3) switches to compact cells above 16 cores so 64 cores render as ~4×16, not a 64-row scroll. Height-capped with scroll as a backstop for 128.
- **Hyperthreading:** cores are logical CPUs as the OS/sysinfo report them; no special handling — `Core N` matches `cpu:N`.
- **Sampling cadence:** panel updates at the `telemetry:interval` rate (default 1s); no extra polling. Per-core values are point-in-time (not smoothed), matching the status-bar aggregate.

---

## 7. Out of scope (possible follow-ups)
- Per-core sparkline/history (the `sysinfo-plot.tsx` Observable Plot view already does time-series; could be linked from a "Details" affordance).
- Per-process CPU attribution (already exists per-block via `blockstats`/`BlockStatsBadge`).
- Temperature / frequency readouts (not collected by the current loop).
- Making GPU/Mem/Net/Disk readouts clickable with their own panels (same pattern; separate spec).

---

## 8. Acceptance criteria
1. Clicking `CPU n%` in the status bar opens a panel anchored above it; clicking again, clicking outside, or pressing Esc closes it.
2. The panel shows one unit per logical core, sorted by index, each conveying live load via fill/color.
3. **Adaptive density:** ≤16 cores → labeled rows; 17–64 → compact cells with visible %; 65+ → computed-size heatmap squares with hover/focus detail. Verified at **8, 32, 64, and 128 cores** (synthesize payloads if needed) — each fits the panel without a long scroll, and 256 degrades gracefully (smaller squares + scroll).
4. Heatmap squares are color-coded by a continuous load ramp with a legend; value is reachable by hover/focus and by screen reader (`aria-label`), never color-only.
5. The header shows the aggregate (matching the status-bar number) and a core count.
6. Values update live (~1s) while the panel stays open; closing tears down the subscription.
7. The panel renders correctly over a browser pane (airspace), flips within the viewport, and stays within the paintable area at every core count / chosen width.
6. No backend changes; no new events/RPCs.
