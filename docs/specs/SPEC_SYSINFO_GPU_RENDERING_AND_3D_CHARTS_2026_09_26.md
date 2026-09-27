# SPEC: GPU-accelerated rendering and 3D charts for Sysinfo

**Date:** 2026-09-26
**Status:** proposed — research and design only, nothing built. §9 lists the
decisions needed before anyone starts.
**Scope:** `frontend/app/view/sysinfo/`.
**Related:**
- `agent3/sysinfo-tighter-margins` (PR #3937, already shipped in this same
  pass): tighter panel margins, axis labels moved to the origin. Separate
  concern from this spec; mentioned only because both came from the same
  "refine sysinfo" request.
- `docs/specs/gpu-and-extended-system-metrics.md` (written earlier the same
  day, status: proposed): GPU **as a monitored resource** — collecting
  utilization/VRAM/temperature from the machine and charting it like CPU/
  Mem/Net. This spec does not repeat that work; §1 explains the split and
  where the two connect.
- `docs/reports/REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md`: why
  small multiples are the base layout and combined/dual-axis charts were
  rejected — this spec's 3D proposal follows the same "opt-in alternate
  view, never a replacement" pattern that report recommended.
- `docs/analysis/sysinfo-architecture-assessment-2026-05-03.md` and
  `docs/specs/sysinfo-continuous-monitor-animation-2026-05-03.md`: the
  prior "canvas/WebGL renderer for the existing 2D charts" assessment
  (rejected as out of proportion) — §4 revisits it with 2026 data and
  reaffirms the same conclusion.
- `frontend/util/gpuutil.ts`, `frontend/app/statusbar/GpuStatus.tsx`,
  `agentmux-cef/src/app/gpu.rs`: the app's existing GPU-**rendering**-
  capability detection (hardware / software / unavailable). Any new WebGL
  feature must consult this, not add a second detector.

---

## 1. "GPU" was asked about two different things, and both are covered here

The request was "explore options for GPU," separately from "explore 3D
charts." Read alone, "GPU" could mean either:

**(a) GPU as data to monitor** — utilization%, VRAM, temperature, like CPU/
Mem/Net. This already has a same-day spec,
`docs/specs/gpu-and-extended-system-metrics.md` (NVIDIA via `nvml-wrapper`
first, AMD via Linux sysfs, a `GpuProvider` trait, new `gpu`/`gpu:mem:used`/
`gpu:temp` WPS keys, new `PlotTypes` entries). It's a good plan — tiered by
vendor coverage, gracefully degrades to "no GPU metrics" rather than
crashing, and reuses the existing `TimeSeriesData` event rather than adding
a new one. **This spec doesn't redo it; recommend implementing it as
written.** One gap worth closing before it ships: pin `nvml-wrapper` and
`hardware-query` to specific versions in the spec itself (it names them
without version numbers beyond "0.10+" for one and none for the other) so
the PR that implements it isn't guessing.

**(b) GPU as chart-rendering technology** — should any Sysinfo chart use
WebGL/GPU rendering instead of the current SVG (Observable Plot)? This is
the part with no prior spec. §4 covers whether the *existing* 2D line charts
should move to it (no); §5–§8 cover the *new* 3D view this spec proposes,
which needs it by definition.

The two connect at exactly one point: once (a) ships, "GPU" becomes a
second candidate dataset for the per-entity-over-time 3D view in §6 — one
GPU's usage over time is a line like any other, but *multiple* GPUs over
time is the same "many parallel series, want the shape across all of them
at once" case that per-core CPU already is. The 3D code sketch in §7 is
written to accept either.

## 2. Research: 3D charts for system/operational monitoring

Summary of 2026 guidance, consistent across sources:

- **Pseudo-3D (decorative) is a settled anti-pattern.** A 3D bar/pie/column
  chart tilted for looks distorts area and length perception — reported
  accuracy loss up to ~50% on length comparisons — and adds a perspective
  gradient, drop shadow, or exaggerated depth that carries no data. For a
  monitoring dashboard, where a quick, accurate read matters, this is
  actively harmful, not neutral decoration.
- **The real distinction is true 3D vs. pseudo-3D.** A 3D bar chart of one
  metric is pseudo-3D (decoration on 2D data). A 3D scatter/surface/contour
  that encodes a genuine third data dimension on the z-axis is a different
  thing — legitimate when the third dimension is real, not when it's
  invented to look more impressive.
- **Where 3D earns its place:** operations-center-style dashboards with
  many simultaneous, genuinely multivariate series, and physically/
  spatially-oriented data (plant layouts, network topology, geospatial).
  None of that describes a single time series.
- **The default recommendation for 2026 dashboards is still 2D** — bar,
  line, scatter — reserving 3D for a genuine third dimension, never for
  making a single metric look more impressive. This matches the dataviz
  skill's own non-negotiable ("never a dual-axis chart... two measures of
  different scale → two charts, small multiples, or indexed to a common
  base") in spirit, though a true 3D surface here isn't a dual-axis chart:
  z is one magnitude scale, x/y are position dimensions (time, core), not
  two competing y-scales. That distinction is what makes §6 defensible
  where a decorative 3D bar chart wouldn't be.
- **For "many parallel series over time" specifically** (exactly what
  per-core CPU is: up to 32 series), the literature converges on two
  concrete techniques, both more legible than a rotatable 3D surface for
  precise reading:
  - **A 2D heatmap** (x = time, y = core index, color = utilization) — the
    standard tool for this exact case (e.g. Brendan Gregg's utilization
    heat maps; a seaborn-based per-core CPU heatmap is a common reference
    implementation built directly on `sar` data). No occlusion, no
    perspective distortion, and a value at any (time, core) cell is read
    directly.
  - **Ridgeline/joyplot** (stacked, overlapping density curves) — better
    suited to comparing *distributions* of load across cores than an exact
    time series of instantaneous values; a weaker fit here than the
    heatmap.
  - **A literal 3D surface/mesh** of the same (time, core, utilization)
    data is the "legacy" option in this specific literature — it *can*
    show the same three dimensions, but occlusion (a spike in front hides
    one behind) and perspective (a tall peak near the camera measures
    differently than the same height further away) make it strictly less
    accurate for reading an exact value than the 2D heatmap of the same
    data.

**Honest conclusion, stated plainly rather than hedged:** for exact-value
reading, a 2D heatmap of the same (time, core, utilization) data is the more
rigorous chart, and would satisfy "does 3D add value" more cheaply than
going to WebGL at all — it can be built with `Plot.cell`/`Plot.raster` in
the library already in use, no new dependency, no GPU-tier gating, and it
composes with everything else the app already knows how to render, ship,
and test. If the goal were purely "answer this with a chart," the heatmap
is the fully-defensible answer to "explore 3D charts" without the word
"3D" actually appearing.

The request was specifically to explore 3D and how to code it, so §6–§8
give a real, buildable 3D chart rather than substituting the heatmap
silently. The recommendation is to build both if effort allows,
default to the heatmap being always visible as an actual `PlotTypes` entry
next to "All CPU" (cheap, no new dependency, immediately useful), and offer
the 3D surface as a further, explicitly opt-in "explore" mode layered on
top of the same data — matching the same "alternate view, never a
replacement" pattern the August combined-chart report already established
for this codebase, and the dataviz skill's own principle that the job of
the data (not visual appeal) should pick the form.

## 3. GPU-accelerated 2D chart rendering: current library landscape (2026)

Researched to answer whether general-purpose GPU/WebGL charting has moved
far enough to reconsider the existing SVG-based line panels.

| Tier | Examples | Where it pulls ahead |
|---|---|---|
| Canvas (CPU-rendered) | Chart.js, uPlot | Handles tens of thousands of points; simple, no GPU dependency |
| WebGL | LightningChart JS, SciChart.js, echarts-gl | Millions of points at 60fps; `echarts-gl` in particular is a thinner, less actively maintained layer bolted onto ECharts' 2D core rather than a from-scratch GPU architecture |
| WebGPU (emerging) | ChartGPU | Tens of millions of points; compute-shader downsampling; newest and least proven of the three tiers |

A commonly cited rule of thumb: under ~100k points, canvas/SVG is fine;
100k–1M favors canvas/WebGL; over 1M needs a GPU-native library or accepts
a performance problem to manage.

**Sysinfo's actual data volume is nowhere near that threshold:** ~120
points per series (`DefaultNumPoints`), redrawn at ≤0.5Hz
(`CHART_UPDATE_INTERVAL_MS`), across at most a handful of panels per pane.
This is thousands, not millions, of points per redraw. The prior assessment
(`sysinfo-architecture-assessment-2026-05-03.md`) called a canvas/WebGL
rewrite "1–2 weeks, fundamentally different code path... out of proportion"
— **nothing in the current data volume or the 2026 library landscape
changes that conclusion.** The ~13% sustained GPU-process cost noted in
`sysinfo-view.tsx`'s `CHART_UPDATE_INTERVAL_MS` comment was font hinting
and `Temporal` date formatting on every 1Hz SVG rebuild, already fixed by
throttling and untracked re-render scoping — not a rendering-technology
problem, and not evidence for revisiting this.

**Recommendation: keep Observable Plot (SVG) for every existing 2D line
panel.** GPU rendering earns its complexity only for the new 3D view below,
where it's required by definition (3D has no non-GPU rendering path in a
browser worth using).

## 4. Where GPU rendering is required: the new 3D view

Given §2's honest weighing, the 3D view is proposed as an **opt-in "3D core
view"** — a new item alongside `"All CPU"` in the Plot Type menu (and,
once `docs/specs/gpu-and-extended-system-metrics.md` ships, `"All GPUs"`
too), not a replacement for the small-multiples default.

**What it shows:** a surface where x = time, y = core index (or GPU index),
z = utilization%, colored by z. Rotatable and zoomable. This is the one
"true 3D" case in §2 that has a real third data dimension, not a decorated
2D chart.

## 5. Rendering technology choice

| Option | Verdict |
|---|---|
| **three.js** (recommended) | Mature (10+ years), the largest ecosystem and community of any WebGL library, ~150KB min+gzip for the core (before any addons), full control over geometry/color/camera/interaction. Requires hand-building the surface mesh, color ramp, and hover/tooltip — more code than a batteries-included charting library, but the amount is small for a single height-field mesh (§7). |
| Plotly.js (`gl3d`, built-in `surface` trace) | Batteries-included: orbit camera, colorscale legend, hover-to-value out of the box. Much less code to write. Cost: the `gl3d` bundle is large (~1.1MB even in the slimmed dist), and Plotly's scene graph is not the kind of thing this codebase would reuse elsewhere — a single-purpose heavy dependency. |
| echarts-gl | Same "thin GPU layer on a 2D-first library" concern noted in §3; would also pull in ECharts core the app doesn't otherwise use. |
| LightningChart JS | Best-in-class WebGL performance (10M points in ~0.3s in vendor benchmarks) — far beyond what this feature needs, and it's a commercial/paid license. Not justified for a few thousand vertices. |
| ChartGPU (WebGPU) | Newest, most exotic option; benchmarks are impressive but it's a young project with WebGPU as its only path (no WebGL fallback). CEF here is pinned to Chromium ~152 (`cef = "152"` in `agentmux-cef/Cargo.toml`), which almost certainly has WebGPU available — but betting a single small feature on the least-proven library in the comparison, when the data volume doesn't need WebGPU's headroom, isn't justified. Worth revisiting only if a future 3D feature actually needs tens of millions of points. |

**Recommendation: three.js**, dynamically imported (see §8) so its cost is
paid only by users who open the 3D view, matching this codebase's existing
care about renderer cost (the `CHART_UPDATE_INTERVAL_MS` throttling, the
debounced `ResizeObserver`, `gpuutil.ts`'s own comment about not stealing a
WebGL context slot from xterm).

**Mandatory integration point — GPU-tier gating.** The app already detects
hardware vs. software vs. unavailable GPU rendering
(`frontend/util/gpuutil.ts`'s `getGpuInfo()`, surfaced today as the "GFX"
status-bar badge). A software-rendered (SwiftShader/llvmpipe) WebGL surface
of ~4,000 vertices redrawn at even 0.5Hz risks being sluggish enough to
actively hurt the experience; an "unavailable" GPU means it can't run at
all. The 3D Plot Type option must:
- Read `getGpuInfo().classification` before offering the menu item.
- Show it, but disabled with a tooltip explaining why, when `"software"` or
  `"unavailable"` — the existing `GpuStatus` tooltip pattern ("Graphics:
  software rendering", "GPU disabled — WebGL unavailable...") is the model
  for the wording.
- Never create a second WebGL capability probe — call `getGpuInfo()`, don't
  reimplement it.

## 6. Design: color, interaction, accessibility

- **Color: one sequential hue, light→dark — never a rainbow height map.**
  This is the dataviz skill's non-negotiable for magnitude encoding, and it
  applies exactly here: z is a single continuous quantity (utilization%),
  not a categorical identity. The app's existing per-metric colors
  (`--sysinfo-cpu-color: #58c142` and friends in `theme.scss`) are
  categorical (CPU vs. Mem vs. Net identity), the wrong kind of ramp for
  this job — reusing one of them as-is would put an identity color where a
  magnitude ramp belongs. Recommendation: derive a light→dark ramp from
  the same hue as `--sysinfo-cpu-color` (or `--sysinfo-net-color` for a
  future GPU surface, keeping each metric's existing hue as its identity
  even in 3D) computed as steps, not eyeballed, and contrast-checked
  against both the light and dark chart surface the same way the dataviz
  skill's validator checks its own reference palette — this needs the
  app's actual surface hex values, which weren't pulled into this pass;
  flagged as a decision in §9 rather than guessed at here.
- **Interaction — a hover layer is not optional.** The dataviz skill's rule
  ("ship a crosshair+tooltip on line/area... the only form that skips it is
  a bare stat tile") adapts to 3D as: raycast from the pointer to the
  nearest mesh vertex, highlight it, and show a tooltip with the exact
  values (core, time, %) — because a 3D surface alone, with no way to read
  an exact number off it, is decoration with extra steps. Orbit (rotate)
  and zoom (scroll) are additive, not a substitute for this.
- **Accessibility.** The skill requires a table view exist for any chart and
  that identity never rests on color alone. Here, both are already
  satisfied for free by keeping this strictly opt-in: `"All CPU"` (the
  existing small-multiples view, fully accessible, no WebGL dependency)
  remains the default and is never removed. The 3D view is a supplementary
  "explore" mode a user turns on, not the only way to see the data — which
  also means a browser/host without WebGL, or a screen-reader user, loses
  nothing that existed before this feature.

## 7. Code sketch

Illustrative, not production-ready — the geometry/color functions are written
as pure functions specifically so they can be unit-tested the same way
`computeAutoMaxY`/`computePlotMargins` are, without needing a WebGL context
in the test environment.

```ts
// sysinfo-surface-util.ts — pure, unit-testable (no three.js import here)

export type SurfaceGeometryInput = {
    /** One row per entity (core or GPU index), each an array of {ts, value}
     *  samples already aligned to the same time grid as the 2D panels. */
    series: Array<Array<{ ts: number; value: number }>>;
    maxZ: number; // same hard-cap/auto-scale value the 2D panels already compute
};

/** Flat vertex positions (x=time-index, y=entity-index, z=value) for a
 *  THREE.BufferGeometry position attribute — kept framework-free so this
 *  function has no three.js dependency and is trivially unit-testable. */
export function buildSurfaceVertices(input: SurfaceGeometryInput): Float32Array {
    const rows = input.series.length;
    const cols = rows > 0 ? input.series[0].length : 0;
    const verts = new Float32Array(rows * cols * 3);
    let i = 0;
    for (let y = 0; y < rows; y++) {
        for (let x = 0; x < cols; x++) {
            const z = Math.min(input.series[y][x]?.value ?? 0, input.maxZ);
            verts[i++] = x;
            verts[i++] = y;
            verts[i++] = z;
        }
    }
    return verts;
}

/** One sequential-ramp color per vertex, light->dark by z/maxZ. `ramp` is a
 *  precomputed array of N hex steps (see §6 — derived from the metric's own
 *  hue, contrast-checked, not invented here). */
export function buildSurfaceColors(zValues: Float32Array, maxZ: number, ramp: string[]): Float32Array {
    const colors = new Float32Array((zValues.length / 3) * 3);
    for (let v = 0, c = 0; v < zValues.length; v += 3, c += 3) {
        const t = maxZ > 0 ? Math.min(zValues[v + 2] / maxZ, 1) : 0;
        const [r, g, b] = hexToRgb01(ramp[Math.round(t * (ramp.length - 1))]);
        colors[c] = r; colors[c + 1] = g; colors[c + 2] = b;
    }
    return colors;
}
```

```tsx
// sysinfo-plot-3d.tsx — the SolidJS component; three.js is dynamically
// imported so it never loads for a user who doesn't open this view.

async function mountSurface(container: HTMLDivElement, input: SurfaceGeometryInput, ramp: string[]) {
    const THREE = await import("three");
    const { OrbitControls } = await import("three/addons/controls/OrbitControls.js");

    const vertices = buildSurfaceVertices(input);
    const colors = buildSurfaceColors(vertices, input.maxZ, ramp);

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute("position", new THREE.BufferAttribute(vertices, 3));
    geometry.setAttribute("color", new THREE.BufferAttribute(colors, 3));
    // ...index into a triangle strip across the (rows x cols) grid, computeVertexNormals()

    const material = new THREE.MeshStandardMaterial({ vertexColors: true, side: THREE.DoubleSide });
    const mesh = new THREE.Mesh(geometry, material);

    const scene = new THREE.Scene();
    scene.add(mesh, new THREE.AmbientLight(0xffffff, 0.6), new THREE.DirectionalLight(0xffffff, 0.6));

    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setSize(container.clientWidth, container.clientHeight);
    container.appendChild(renderer.domElement);

    const camera = new THREE.PerspectiveCamera(45, container.clientWidth / container.clientHeight, 0.1, 1000);
    camera.position.set(0, -input.series.length, input.maxZ * 2);
    const controls = new OrbitControls(camera, renderer.domElement);

    // Raycast hover -> tooltip, mirroring SingleLinePlot's Plot.tip/pointerX role.
    const raycaster = new THREE.Raycaster();
    renderer.domElement.addEventListener("pointermove", (e) => {
        // ...pick nearest vertex, show {core, time, value} in a tooltip element
    });

    let raf = 0;
    const loop = () => {
        controls.update();
        renderer.render(scene, camera);
        raf = requestAnimationFrame(loop);
    };
    loop();

    return () => {
        // Mirrors SingleLinePlot's onCleanup: WebGL contexts are a capped,
        // shared resource (gpuutil.ts's own comment on the ~16-context
        // Chromium limit) — dispose explicitly, don't rely on GC.
        cancelAnimationFrame(raf);
        controls.dispose();
        geometry.dispose();
        material.dispose();
        renderer.dispose();
        container.removeChild(renderer.domElement);
    };
}
```

The SolidJS wrapper around `mountSurface` follows `SingleLinePlot`'s existing
shape exactly: a debounced `ResizeObserver` (150ms, reusing the same
constant sysinfo-plot.tsx already has), an `onMount`/`onCleanup` pair calling
the returned disposer, and a `createEffect` gated on `getGpuInfo().classification === "hardware"`.

## 8. Bundle size and load cost

`three` core is ~150KB min+gzip; `OrbitControls` adds a few KB more. Neither
should enter the main bundle for every user — **dynamic `import()`** (as in
the sketch above), triggered only when the 3D Plot Type is actually
selected, keeps the cost off everyone who never opens it. Vite (already the
build tool here, per `frontend/vite.config.ts`) code-splits a dynamic
import into its own chunk automatically — no extra configuration needed
beyond writing the `import()` call itself.

## 9. Decisions needed before building

1. **Priority.** Build the 2D heatmap (§2, cheap, no new dependency) first,
   the 3D surface second, or skip straight to 3D? Recommend the heatmap
   first — it's a `PlotTypes` entry and a couple of new marks in the
   existing `sysinfo-plot.tsx`, shippable in under a day, and is the more
   rigorous answer per §2.
2. **Color ramp source.** Pull the app's actual light/dark chart-surface hex
   values and run them (or an equivalent contrast check) against a
   `--sysinfo-cpu-color`-derived sequential ramp before shipping the 3D
   view's color scale — not guessed at in this pass.
3. **Sequence with the GPU-metrics spec.** Should the 3D view ship scoped to
   CPU cores only, or wait for `gpu-and-extended-system-metrics.md` so
   `"All GPUs"` can reuse the same component from day one?
4. **Three.js version/addons path.** Confirm the exact `three` version and
   whether `OrbitControls` should come from `three/addons/...` (current) or
   `three/examples/jsm/...` (older import path) at implementation time —
   this drifts across three.js releases and should be checked against
   whatever version is actually installed, not assumed from this spec.

## Sources

- 3D vs. pseudo-3D, when 3D adds value, 2026 dashboard defaults:
  https://www.domo.com/learn/charts/3d-charts ,
  https://www.fanruan.com/en/blog/3d-data-visualization ,
  https://thedan.design/insights/dashboard-design-principles-best-practices-to-enhance-your-data-analysis/
- Per-core CPU heatmaps and ridgeline/joyplots as the established
  alternative: https://www.brendangregg.com/HeatMaps/utilization.html ,
  https://github.com/Kadle11/CPU_Heatmap ,
  https://www.data-to-viz.com/graph/ridgeline.html
- GPU/WebGL charting library landscape and performance tiers (2026):
  https://www.scichart.com/blog/chart-bench-compare-javascript-chart-libraries/ ,
  https://lightningchart.com/blog/the-ultimate-javascript-charting-library-comparison-2026/ ,
  https://www.webgpu.com/showcase/chartgpu-webgpu-charts/
- Observable Plot margins/labelAnchor (used in the companion PR #3937, not
  this spec, but from the same research pass):
  https://observablehq.com/plot/features/plots ,
  https://github.com/observablehq/plot/blob/main/src/plot.d.ts

Web sources are secondary summaries, current as of this research pass
(2026-09-26); re-check library versions and benchmarks at implementation
time rather than trusting the numbers here to still be current.
