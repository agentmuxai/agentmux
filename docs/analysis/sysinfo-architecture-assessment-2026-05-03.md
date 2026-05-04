# Sysinfo Architecture Assessment

**Date:** 2026-05-03
**Files reviewed:**
- `frontend/app/view/sysinfo/sysinfo-plot.tsx` (159 LOC)
- `frontend/app/view/sysinfo/sysinfo-model.ts` (259 LOC)
- `frontend/app/view/sysinfo/sysinfo-view.tsx` (108 LOC)
- `frontend/app/view/sysinfo/sysinfo-types.ts` (113 LOC)
- `frontend/app/view/sysinfo/sysinfo-util.ts` (36 LOC)
- `frontend/app/view/sysinfo/sysinfo.tsx` (17 LOC)
- **Total: 692 LOC**

**Triggering question:** is the original sysinfo architecture worth improving or replacing?

## TL;DR

Mid-tier code. Works fine, has the smell of "feature shipped, never came back to." **Do not replace** — the cost-to-value ratio is bad. **Improve selectively** — three high-value moves listed below; everything else can stay.

## What's good

- Clean module separation: model / view / plot / types / util — small focused files
- Solid migration done properly: ViewModel class, signal atoms, `createMemo` for derived state, `onCleanup` everywhere
- Uses `@observablehq/plot` (Mike Bostock's library — D3 author) — legitimate dependency, not a homegrown wheel
- `DataItem` index-signature shape (`[k: string]: number`) is extensible without schema changes
- `PlotTypes` lookup table is data-driven; adding a new plot view is one entry
- Gap-detection logic (blank-item insertion in `addInitialData`/`addContinuousData`) is thoughtful
- Resize observer + reactive plumbing has nothing janky

## What's not good (concrete issues)

| # | Issue | Location | Severity |
|---|---|---|---|
| 1 | Destroy-and-recreate hot path: `createEffect` wipes containerRef and calls `Plot.plot()` from scratch on every data change (~1Hz per pane) | sysinfo-plot.tsx:42–154 | Medium (perf concern at scale; root of why animation is hard) |
| 2 | `try/catch` with `console.log` swallows errors silently | sysinfo-model.ts:73, 88 | Low (should be `console.error`) |
| 3 | `model: any` to dodge circular dep | sysinfo-types.ts:16 | Low (fixable with `import type`) |
| 4 | `data.push(newPoint)` mutates atom's underlying array before filter | sysinfo-model.ts:84 | Low (referential checks downstream could mislead) |
| 5 | `_set` on the atom — internal API leaking | sysinfo-model.ts:71, 86 | Low (smell) |
| 6 | Hardcoded 32-CPU loop | sysinfo-types.ts:111 | Low (silently broken for 64+ core machines like Threadripper Pro) |
| 7 | `htl.svg` template literal mixes raw HTML strings into a Solid component | sysinfo-plot.tsx:61–67 | Low (off-grain but works) |
| 8 | `SingleLinePlot` renders 6 marks (line + area + gradient + tooltip + dot + rules) | sysinfo-plot.tsx:25 | Trivial (naming) |

The destroy-and-recreate pattern (#1) is the only structural issue. The rest are surface smells.

## Big picture comparison

Compare to what we shipped in the reducer migration (slices #1, #4, #6 — explicit invariants, immutable state, audit events, table-driven tests). Sysinfo doesn't show that level of consideration. It feels like a sprint-shipped feature that got parked. **That's not damning — most code in any sufficiently large codebase looks like this.** It just means the bar set elsewhere in this project is higher.

## Replace vs. improve

### Replace? **No.**

Replacement options I considered:

- **Migrate to a different chart library** (chart.js, uplot, lightweight d3): 3–5 days; throw away ~500 LOC of working code; revisit visual correctness across 9 plot types and many CPU configs; high risk of subtle regressions. **Marginal user-visible benefit.**
- **Custom canvas/WebGL renderer**: 1–2 weeks; fundamentally different code path; future maintainers need WebGL knowledge. **Out of proportion to benefit.**
- **Rewrite the destroy-and-recreate effect to use Plot's update API**: doesn't exist — `@observablehq/plot` doesn't expose in-place updates. Would require switching libs.

None of these clear the bar.

### Improve? **Yes — selectively.**

Three high-value moves, in priority order:

| Improvement | Effort | Value | Status |
|---|---|---|---|
| 1. Continuous-monitor animation | 1–1.5d | High visual quality bump; restructures the destroy-and-recreate hot path | Spec written: `sysinfo-continuous-monitor-animation-2026-05-03.md` |
| 2. Cleanup pass: `console.error`, `model: any`, `data.push` mutation, dynamic CPU count | ~1h | Removes the smells; trivially safe | Not started |
| 3. Cache the Plot instance + only re-render when the data domain actually changes (vs. on every signal change) | 0.5d | Real CPU win on machines with many sysinfo panes; speculative until measured | Not started |

### Skip

| Idea | Why not |
|---|---|
| Migrate off `@observablehq/plot` | Cost ≫ benefit; current lib is reasonable |
| Rewrite the model in the reducer pattern | Sysinfo state is per-pane and read-mostly; no multi-writer hazard. Reducer pattern's value prop is invariant enforcement against multiple writers — doesn't fit here. |
| Add tests retroactively | Data-handling logic is small; visual surface is hard to test meaningfully. ROI is low. |

## Net call

If the user wants monitoring polish, do the animation (improvement #1). The 1-hour cleanup pass (#2) is worth doing whenever bandwidth allows. The plot-instance caching (#3) is speculative — defer until perf is actually measured to be a problem.

The architecture is **good enough to leave alone for now** unless one of these three pulls is taken on. Anything beyond is gold-plating.
