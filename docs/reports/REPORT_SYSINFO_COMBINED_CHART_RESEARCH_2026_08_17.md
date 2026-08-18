# Report: Combining CPU/Memory/Network into one Sysinfo chart — research + recommendation

**Date:** 2026-08-17
**Status:** Investigation complete — no code changed. Written to inform a design decision before implementation.
**Scope:** `frontend/app/view/sysinfo/` (`sysinfo-view.tsx`, `sysinfo-plot.tsx`, `sysinfo-model.ts`, `sysinfo-types.ts`).

---

## Ask

The sysinfo widget already has a "CPU + Mem + Net" plot type with "some shared axis and different colors." Research best practices for bringing CPU/memory/network into a single chart.

## What the widget actually does today

The "combined" plot types (`"CPU + Mem"`, `"CPU + Mem + Net"` in `sysinfo-types.ts`'s `PlotTypes`) don't render as one chart. `sysinfo-model.ts`'s `metrics()` returns an array of metric keys for the selected plot type (e.g. `["cpu", "mem:used", "net:bytestotal"]`), and `sysinfo-view.tsx`'s `<For each={yvals()}>` mounts **one independent `SingleLinePlot` per metric** — each is its own `Plot.plot()` call with its own SVG, own `Plot.lineY`/`Plot.areaY` marks, own gradient, own tooltip. They're laid out in a CSS grid (`sysinfo-view.tsx:96-100`) with `title={true}` labeling each panel's metric name.

What reads as "shared" between them is real but limited to two things:
1. **Same time (x) domain** — every panel receives the same `plotData`/`targetLen`/`intervalSecs`, so their x-axes line up even though each is a separate SVG (`sysinfo-plot.tsx:160-161`).
2. **Distinct default colors per metric** — `--sysinfo-cpu-color`, `--sysinfo-mem-color`, `--sysinfo-net-color` (`sysinfo-types.ts:17,28,39`).

There is no shared *y*-axis, no single coordinate space, and no cross-panel synchronized hover — each panel's `Plot.pointerX` crosshair/tooltip (`sysinfo-plot.tsx:124-156`) only reacts to hovering that one panel. This is a **small-multiples** layout, not a combined chart, despite the plot-type name implying otherwise.

## The core problem: three different units, three different scales

- CPU: 0–100%, already normalized
- Memory: GB, absolute, `maxy` bound to `mem:total` (varies per machine — 8GB, 32GB, 128GB...)
- Network: MB/s, absolute, unbounded burst range (idle near 0, spikes can be 10-100x baseline)

Putting all three literally on one y-axis is meaningless — a memory line sitting at "16" (GB) and a CPU line sitting at "45" (%) share no unit, and network's MB/s range doesn't remotely align with either. Any single-chart approach has to solve this scale mismatch first; the visual combination is secondary.

## Options researched

### 1. Dual/multi-axis (a `y2` secondary scale)

Observable Plot (the library already in use here) supports a `y2` scale anchored to the right side of a chart, letting one chart show two independently-scaled series ([Plot: Dual axis chart](https://observablehq.com/@observablehq/plot-dual-axis), [Plot scales docs](https://observablehq.com/plot/features/scales)). Technically feasible for *two* series. This widget needs *three* (CPU/Mem/Net) — Plot doesn't have a `y3`, so a third series would need a manually-normalized value plotted against `y` or `y2`, defeating the point of a "real" axis for it.

Data-visualization sources are consistently cautious about dual axes even for the two-series case:
- Different axis baselines/scales mean **crossing lines are meaningless** — where a CPU line and a memory line happen to cross visually says nothing about their actual relationship ([Flourish](https://flourish.studio/blog/dual-axis-charts/), [Datawrapper](https://www.datawrapper.de/blog/dual-axis-charts-guide)).
- Best-practice guidance when dual axes are used anyway: explicit per-axis labels+units, distinct colors, primary/more-important series on the left axis, and normalized scaling to avoid exaggerating one series ([Inforiver](https://inforiver.com/insights/dual-axis-charts-101-introduction-best-practices/), [Nir Smilga](https://smilganir.medium.com/dual-axes-suggested-dos-and-don-ts-39b9cc60c475)).
- General recommendation: only reach for dual axes when there's no better alternative, precisely because they're easy to misread.

**Verdict:** technically possible for 2 of the 3 metrics, structurally awkward for 3, and the exact pattern the visualization literature warns against defaulting to.

### 2. Normalize everything to a shared 0–100 scale ("index chart")

Convert each series to a percentage of its own max (or, for CPU, use the value as-is since it's already 0-100%) and plot all three on one shared y-axis in relative terms. This is the standard technique for comparing differently-unitless time series ("indexed to 100" — [eagereyes](https://eagereyes.org/blog/2020/eagereyestv-index-charts-part-1-making-time-series-data-comparable), [Dallas Fed](https://www.dallasfed.org/research/basics/indexing)), and it's exactly what real monitoring tools do for multi-core CPU: CockroachDB's hardware dashboard and Grafana's per-node Kubernetes dashboards both normalize CPU to a 0-100% "utilization" figure across cores before charting it ([CockroachDB docs](https://www.cockroachlabs.com/docs/stable/ui-hardware-dashboard), [Kubernetes/Grafana dashboards](https://oneuptime.com/blog/post/2026-02-20-kubernetes-grafana-dashboards/view)).

Applied here: CPU is free (already %). Memory would plot as `mem:used / mem:total * 100`. Network is the hard case — there's no natural "100%" ceiling for throughput (a home connection's 100 Mbps and a datacenter NIC's 10 Gbps are both "network usage," but neither is an inherent max); it would need an arbitrary or rolling-window-relative ceiling (e.g. normalize against the observed peak in the visible window), which is less honest than CPU%/Mem% and could make a real spike look identical to a much smaller one on a different day.

**Verdict:** clean and legitimate for CPU+Mem. Network resists honest normalization without picking an arbitrary reference ceiling — workable but the weakest fit of the three.

### 3. Small multiples (the status quo, done properly)

Tufte's own conclusion, and the consistent recommendation across data-viz sources, is that small multiples are usually the *better* answer to "I have several differently-scaled series to compare over the same time axis" — not a fallback when a combined chart isn't achievable ([Forum One](https://www.forumone.com/insights/blog/good-data-visualization-practice-small-multiples/), [Pew Research](https://www.pewresearch.org/decoded/2018/12/20/how-pew-research-center-uses-small-multiple-charts/)): each panel keeps its own honest, unambiguous scale; the shared x-axis (time) is what actually needs to line up for the comparison to be meaningful, and it already does here.

This is also the design every mainstream OS resource monitor already converged on independently: **Windows Task Manager, macOS Activity Monitor, htop, and Windows Resource Monitor all render CPU/Memory/Network/Disk as separate panels/graphs, never combined into one chart** ([Windows Central](https://www.windowscentral.com/how-use-windows-10-task-manager-monitor-system-performance), [ghacks Resource Monitor guide](https://www.ghacks.net/2017/12/28/a-detailed-windows-resource-monitor-guide/)). This isn't a limitation of those tools — it's the converged-on answer for exactly this dashboard shape, by every major platform.

**What's actually missing today isn't a combined chart — it's that the small multiples don't yet behave like a *coordinated* set.** Concretely:
- Hovering one panel doesn't move a synchronized crosshair on the others (each `Plot.pointerX` is scoped to its own panel, `sysinfo-plot.tsx:124-156`).
- The panels are visually independent SVGs, not a single logical group — nothing currently reinforces "these three update together" beyond proximity in the grid.

**Verdict:** already the right shape per the research; the improvement opportunity is *coordination between panels*, not *merging them into one*.

## Recommendation

**Keep small multiples as the base — this is not a regression to fix, it's the industry-converged answer.** Two concrete follow-ups worth doing, in order of value:

1. **Synchronized crosshair across panels.** Lift the hover/pointer-x state up to `SysinfoViewInner` (or a small shared signal) so moving the mouse over any one panel draws the same vertical time-cursor on all panels in the current plot-type group, with each panel's own tooltip showing its own value at that instant. This is what makes small multiples *feel* combined without sacrificing per-metric scale honesty — genuinely the missing piece, not a compromise.
2. **For "CPU + Mem" specifically** (2 series, both nameable as %-of-something), consider an *opt-in* normalized overlay view using the indexed-to-100 technique from Option 2 above — CPU as-is, Memory as `used/total * 100`. This is defensible because both series have a real, non-arbitrary 100% ceiling. Present it as an alternate view alongside (not replacing) the small-multiples default, since some users will want the absolute GB figure at a glance.
3. **Do not build a literal dual/triple-axis combined chart** for "CPU + Mem + Net" — three units, one of them (network) with no honest normalization ceiling, is exactly the case the dual-axis literature warns against.

No code changes made in this pass. If #1 (synchronized crosshair) is the direction to go, it's a contained change scoped to `sysinfo-plot.tsx`'s pointer marks + a new shared hover-position signal threaded from `sysinfo-view.tsx` — happy to spec or implement on request.

## Sources

- [Dual axis charts: why they spark debate, and how to get them right — Flourish](https://flourish.studio/blog/dual-axis-charts/)
- [Introduction & Best Practices: Dual-Axis Charts — Inforiver](https://inforiver.com/insights/dual-axis-charts-101-introduction-best-practices/)
- [What to consider when creating dual-axis charts — Datawrapper Blog](https://www.datawrapper.de/blog/dual-axis-charts-guide)
- [Dual Axes: Suggested do's and don'ts — Nir Smilga](https://smilganir.medium.com/dual-axes-suggested-dos-and-don-ts-39b9cc60c475)
- [CockroachDB Hardware Dashboard docs](https://www.cockroachlabs.com/docs/stable/ui-hardware-dashboard)
- [Essential Grafana Dashboards for Kubernetes Monitoring](https://oneuptime.com/blog/post/2026-02-20-kubernetes-grafana-dashboards/view)
- [Plot: Dual axis chart — Observable](https://observablehq.com/@observablehq/plot-dual-axis)
- [Scales | Plot (Observable Plot docs)](https://observablehq.com/plot/features/scales)
- [Plot facets with varying scales? — Observable Forum](https://talk.observablehq.com/t/plot-facets-with-varying-scales/5557)
- [Good Data Visualization Practice: Small Multiples — Forum One](https://www.forumone.com/insights/blog/good-data-visualization-practice-small-multiples/)
- [How Pew Research Center uses small multiple charts](https://www.pewresearch.org/decoded/2018/12/20/how-pew-research-center-uses-small-multiple-charts/)
- [Small multiple — Wikipedia](https://en.wikipedia.org/wiki/Small_multiple)
- [Index charts, part 1: Making time series data comparable — eagereyes](https://eagereyes.org/blog/2020/eagereyestv-index-charts-part-1-making-time-series-data-comparable)
- [Indexing data to a common starting point — Dallas Fed](https://www.dallasfed.org/research/basics/indexing)
- [How to use Windows 10 Task Manager to monitor system performance — Windows Central](https://www.windowscentral.com/how-use-windows-10-task-manager-monitor-system-performance)
- [Windows Resource Monitor guide — ghacks](https://www.ghacks.net/2017/12/28/a-detailed-windows-resource-monitor-guide/)
