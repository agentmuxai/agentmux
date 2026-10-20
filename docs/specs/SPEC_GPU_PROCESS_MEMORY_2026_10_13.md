# Spec: why the GPU process holds 394 MB in the field, and what stops it

**Date:** 2026-10-13
**Status:** proposed. Measure first (§3), then fix what the measurement names (§4).
**Plan item:** #11 of `PLAN_RENDERER_MEMORY_FIXES_2026_10_12.md`.
**Evidence:** `docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md` §4.5.

## 1. The problem

| | GPU process private working set | GPU process commit |
|---|---:|---:|
| Operator's live instance (v0.59.20, 26 h uptime) | 394 MB | about 1 GB |
| Lab instances, up to 12 agent panes over 4 tabs | 81–98 MB | 410–445 MB |

The live instance is one 2689×1470 window at 125%, with 4 tabs holding 31 panes:
- 12 agent panes;
- 5 Swarm and 4 Sysinfo panes;
- 3 browser panes and 2 media panes;
- an editor, Settings, Tower and Warden.

The lab had agent panes only. Its GPU process memory-infra dump (CDP `Tracing.requestMemoryDump`) was about `malloc` 100 MB, `cc` 65 MB, `gpu` 108 MB and `skia` 11 MB.

## 2. What in the code creates GPU-process work and memory

### Never told it's hidden

**Browser panes** are windowed CEF browsers in child windows (`crates/cef/src/browser_pane/creation.rs:229`), each with its own compositor surface and raster tiles in the GPU process.
- Hiding one collapses its rect to 0×0 (`frontend/app/view/browser/use-pane-rect-sync.ts:141` → `crates/cef/src/browser_panes/bounds.rs:303-348`, `DeferWindowPos`). There is no `SW_HIDE`.
- `was_hidden` is never called anywhere in `crates/cef/src`.
- Native occlusion detection is off: `--disable-features=CalculateNativeWinOcclusion`, `crates/cef/src/app/mod.rs:638,650`, added for blank pre-warmed windows (CEF #3638). `docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md` says it didn't fix that.
- So Chromium never treats a hidden browser pane, or an occluded AgentMux window, as hidden. Its page keeps running `requestAnimationFrame`, video and timers, and its frames are never evicted.

### Keeps animating in hidden tabs

Inactive window tabs stay laid out by default (`window:keepinactivetabslaidout`; `frontend/app/workspace/window-tab-visibility.ts:24-26, 98-110`), hidden only by `visibility: hidden`. These don't check visibility:

| What | Where | Why it costs |
|---|---|---|
| **Sysinfo** | `frontend/app/view/sysinfo/sysinfo-view.tsx:97-100`, `sysinfo-plot.tsx:105-286` | Rebuilds every chart's SVG every 2 s, sometimes twice per update. One plot per core for "All CPU". |
| **The agent pane's busy ring** | `frontend/app/view/agent/agent-view.scss:324-337, 369-394`, `agent-view.tsx:1273-1277` | A masked element whose `::before` is about 2.5× the pane in each dimension, promoted with `will-change: transform`, rotating forever while the pane is busy, in any tab. |
| **`AsciiSpinner`** | `frontend/app/element/AsciiSpinner.tsx:24-41` | A shared 120 ms interval while any spinner is mounted: agent footers, every non-idle Swarm chip. |
| **Media `<video>`** | `frontend/app/view/media/media-view.tsx:473-480` | Keeps its decoder and frame pool while hidden. |
| **xterm WebGL** | `frontend/app/view/term/termwrap.ts:839-849` | One context per terminal, never paused. Two probe contexts are created and never released (`termwrap.ts:40-48`, `frontend/util/gpuutil.ts:51-52`). |
| **Other infinite CSS animations** | `_control-bar.scss:218,343`, `_shell-node.scss:20,189`, `_composer-strip.scss:499`, `BrainSpinner.scss:18`, `StatusBar.scss:106`, `browser-view.scss:112` | Run whenever their state applies. |

### Layers and textures

- **Whole-window layer:** `frontend/app/app.scss:39-40` gives `body` `transform: translateZ(0)`.
- **Per-pane layers:** `will-change: transform` on `.block-mask` (`block.scss:273`), `PaneChrome.scss:64` and the tab bar (`tabbar.scss:53-54`).
- **Extra textures from `backdrop-filter`:** in every agent pane's tab strip (`agent-view.scss:184`, `blur(2px)`), plus `_document-nodes.scss:2760`, `block.scss:117,140,209`, `tilelayout.scss:169`, `modal.scss:84`, `pane-size-badge.scss:24`.
- **Tab switches:** `document.startViewTransition()` makes full-window snapshot textures (`frontend/app/workspace/workspace.tsx:49-85`).

### No budget

No GPU memory flags are set (no `force-gpu-mem-available-mb`). The compositor's tile budget scales with display area, and this window is large at 125%.

## 3. Measure first

Which of these explains the 394 MB isn't known. Cutting the wrong one wastes the effort.

**Reproduce the field layout in the lab:**
- an isolated production portable (the root-cause report's method), with the window sized to 2689×1470 at 125%;
- add pane groups one at a time: 3 browser panes on live sites (one with video), 2 media panes (one playing video), 4 Sysinfo, 5 Swarm, and busy agents in hidden tabs.

**After each step, record:**
- the GPU process's private working set and commit (OS);
- its memory-infra dump by allocator (`memlab/memsnap.mjs`: `gpu/gl`, `gpu/shared_images`, `cc`, `skia`);
- GPU-process CPU, and the main renderer's long animation frames.

**Then check how it changes over time:**
- 2 hours with that layout, sampled every 10 minutes, to separate steady per-pane cost from growth;
- if the dump's GPU allocators don't explain the private working set, measure from outside with ETW (`Microsoft-Windows-DxgKrnl`), as `SPEC_GPU_MEMORY_TRACING_SCAFFOLDING_2026_07_24.md` §6 suggests. CEF's own tracing returned no size data there.

**Output:** a table of GPU memory per pane type, and per hour if it grows, appended to this spec.

## 4. Fixes, chosen by what §3 finds

In expected-impact order:

1. **Hide browser panes properly.**
   - When a browser pane is hidden (inactive pane tab, inactive window tab, minimized window), call `BrowserHost::was_hidden(true)` and `SW_HIDE` its window; reverse both on show.
   - Chromium then stops its frames, throttles its timers and can evict its tiles.
   - This is the browser-pane half of what native occlusion would have done.
2. **Pause hidden work.** Gate on the existing `usePaneTabVisibility` and a window-tab visibility signal:
   - Sysinfo's chart rebuilds;
   - the busy ring's animation (a static state when hidden);
   - spinners (their interval stops while no visible spinner is mounted);
   - media `<video>`: pause it and release its source after a minute hidden.
3. **Re-test native occlusion.**
   - Remove `CalculateNativeWinOcclusion` from the disabled features on a dev build and run the pre-warmed-window blank repro from the 2026-06-21 research doc.
   - If the blank doesn't come back, keep occlusion on: occluded windows then stop producing frames, which Chrome measured as about −3% GPU memory and −20% frames.
4. **Drop the layers nothing needs:**
   - `body`'s `translateZ(0)`;
   - `will-change` on elements that don't animate;
   - `backdrop-filter` in the always-visible agent tab strip (a solid tint looks the same at `blur(2px)`);
   - release the two probe WebGL contexts (`WEBGL_lose_context`).
5. **Set a tile budget** with `force-gpu-mem-available-mb`, only if §3 shows tile memory dominating, after checking rendering on a large high-DPI display. One app measured −50 MB at peak and then reverted it over high-DPI artifacts.

**Not changed here:** xterm WebGL policy (visible terminals only, atlas caps), which is a terminal item of its own.

## 5. Verification

- Rerun §3's layout after each fix; record GPU process memory and CPU.
- Check that hidden browser panes resume correctly: video, scroll position, a page mid-load.
- Check that Sysinfo catches up on show (its data subscription stays live).
- Check that the busy ring and spinners resume.
- Check that occlusion on doesn't blank pre-warmed windows (§4.3).
