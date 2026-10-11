# Linux window paint: what the Windows and macOS work gives Linux, and what Linux still needs

**Date:** 2026-10-11
**Status:** analysis. §2 is what carries over, §3 the Linux measurements, §4 the causes, §5 the recommendations. Nothing here changes product code.
**Author:** Opaz
**Trigger:** Repo owner, 2026-10-11: *"take a look at the recent performance improvement work regarding window painting and quick responsiveness during window resize. we did a bunch of work on windows and macos. can any be used for performance gains on linux? take a look, write a report to file"*
**Related:** `ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md` (Windows), `ANALYSIS_MACOS_WINDOW_PAINT_2026_10_10.md` (macOS), `ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md`, `SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md`.

Code citations are against `main` @ `cbb2837a8`. Measurements use the macOS analysis's scripts (`scripts/perf/macos-paint/`), which run unchanged on Linux with `--port`.

---

## 1. Summary

- **Most of the work already runs on Linux, and works.** The resize and tab-switch changes are shared frontend code. Measured on Linux, the skip-hidden-tabs change (#4396) does what it did on Windows and macOS: an empty visible tab with 16 terminals in hidden tabs renders 80–91 frames of ~75 during a stepped resize, the same as an empty window.
- **Linux is in better shape than macOS before its fixes.** A visible tab with 4 terminals renders 67–71 frames of ~75 (macOS: median 51 of ~83). A tab of 4 browser panes renders 76–84 frames, and each batch of pane rects is applied in 9.3 ms (p50).
- **The host-side changes did not reach Linux, and two of them matter here:**
  - **Browser-pane geometry.** #4420 batched pane rects per window, and Windows then applies them in one deferred move. #4657 gave macOS a frame-only path for panes already showing. On Linux the batch arrives, but the host still runs a full `set_size` + `set_position` + `set_visible` + `window.layout()` **for each pane** (`browser_panes/bounds.rs:179-181`, `browser_pane/creation_views.rs:563-580`). That is one whole-window Views layout per pane per batch.
  - **Menus over browser panes.** Linux still hides the whole pane and shows a screenshot of it while a menu overlaps it. The screenshot takes 34–190 ms (median ~170 ms), and the menu waits for it (up to 300 ms). macOS replaced that path with a hole-punch mask.
- **The remaining cost is the same as on macOS:** terminals' WebGL drawing buffers resized synchronously with the GPU process. In a trace, the terminals' live refits took 218 ms of wall time but 66 ms of CPU in a 1.4 s drag, even with #4658's time budget.
- **The WebGL context-loss reload loop (macOS M4) reproduces on Linux.** With 19 terminals the page reloaded itself repeatedly ("context lost — reloading page", 9 times in the log).

## 2. What carries over to Linux

| Change | Where it runs | On Linux |
|---|---|---|
| #4396: hidden tabs skip layout while the window resizes; tile layout takes the ResizeObserver's size | Shared | **Works.** §3.2 |
| #4399: terminals refit every frame, only the PTY size waits; System Info timer removed | Shared | Runs. Now the main cost (§4.1) |
| #4404, #4406: `animate` class, plot styles, tab strip read, Tailwind width media queries | Shared | Applied. No viewport media queries on the page (`env.mjs`: `mediaQueries: false`) |
| #4408, #4658: terminal refits take turns, with a time budget | Shared | Runs (§4.1) |
| #4415: agent list measures rows in one pass | Shared | Applied, not separately measured |
| #4107, #4108: one-frame warm tab switch, new tab built in parallel | Shared | Applied, not separately measured |
| #4398: no `windowresize:tick` without Shift, one position writer, WinEvent log demoted | Windows host | **Nothing to port.** The Linux host has no resize hook, no `windowresize:*` events and no position stream (`wrr::install_hooks` is a stub off Windows, `wrr/mod.rs:34-40`). On Wayland, Mutter drives the interactive resize. |
| #4420: one `browser_panes_set_rects` per window per frame | Frontend shared; host apply per platform | **Half applied.** The batch arrives; the Linux host then resizes and lays out the window once per pane (§4.2) |
| #4657: macOS frame-only update for a pane already showing | macOS host | **No Linux counterpart** (§4.2) |
| #4688: macOS edge resize with a browser pane open | macOS host | Not applicable |
| #2968: startup paint gate | Windows and Linux | Already on Linux (`client/navigation.rs:12`) |
| Pane overlay hole-punch mask (`ui_tasks/pane_hole_mask.rs`) | macOS host | **No Linux counterpart.** Linux hides the whole pane and shows a freeze-frame (§4.3) |
| `notifyPaneReflow` on a leaf geometry change | `TileLayout.win32.tsx:82` only | **Not wired on Linux** (or macOS). A pane that moves without resizing waits for the 200 ms safety poll (§4.4) |

## 3. Measurements on Linux

### 3.1 Setup and limits

- **Machine:** charlie, a VMware VM, GNOME 50 on Wayland, 1920×1080, DPR 1, 60 Hz (idle frame gap 16.7 ms).
- **GPU:** VMware SVGA3D through Mesa 26.0.8, ANGLE on OpenGL (`--use-angle=gl`). GPU compositing and rasterization on; Skia Graphite and Vulkan off.
- **Build:** `task dev` of `main` @ `cbb2837a8` (0.59.20), CEF 154 with the agentmux patches, isolated home, CDP on 9223.
- **Driver:** `resize-frames.mjs`: the host's `set_window_rect` 80 times, 10 px every 16 ms, shrinking then growing, as in the macOS analysis. On Wayland the window's size applies and its position is ignored, which this test doesn't need. **It is not a mouse drag**, which on GNOME goes through Mutter's interactive resize.
- **Load:** 1-minute load average 2.0–3.2.
- **Not measured:** a real mouse drag; how far native panes trail the window edge in pixels; real GPU hardware; X11 sessions.

### 3.2 Frames during the stepped resize

"Ideal" is driver time ÷ 16.7 ms. "Sizes seen" is how many of the 80 widths reached the page.

| Case | Runs | Frames (ideal ~75) | Gap p50 / p90 / p99 ms | Long frames | Sizes seen |
|---|---|---|---|---|---|
| Empty window, 1 tab | 3 | 76–105 | 11.4–19.9 / 16.7–23.1 / 23.3–40.6 | 0 | 39–41 |
| **Empty visible tab, 16 terminals in 4 hidden tabs** | 3 | **80–91** | 12.2–16.7 / 22.0–22.9 / 24–44.6 | 0–1 | 36–37 |
| **Visible tab with 4 terminals** (+12 hidden) | 4 | **67–71** | 16.7–17.7 / 24.4–32.8 / 63.5–78.2 | 2–3 (131–186 ms) | 24–27 |
| Same, while tracing | 1 | 56 | 24.4 / 38.4 / 74.9 | 3 | 22 |
| Visible tab with 4 browser panes | 3 | 76–84 | 12.7–16.7 / 23.7–24.5 / 36–70 | 2 (106–160 ms) | 28–34 |

Frame gaps below 16.7 ms appear on Linux only: during a resize Chromium on Wayland produces frames faster than the display's 60 Hz, likely one per configure. That is harmless.

### 3.3 Where the time goes (trace of a 1.4 s resize, 4 visible terminals)

| Thread | Wall busy | CPU busy |
|---|---|---|
| Renderer main | **96%** | 81% |
| GPU process main | 36% | 35% |
| Browser UI thread | 14% | 12% |

Top-level JavaScript on the renderer main thread:

| Function | Calls | Wall | CPU |
|---|---|---|---|
| `term-resize-policy.ts` (live refits) | 55 | **218 ms** | **66 ms** |
| `sysinfo-plot.tsx` | 26 | 74 ms | 71 ms |
| `use-widget-bar-responsive.ts` | 30 | 63 ms | 58 ms |
| `term.tsx` | 104 | 58 ms | 33 ms |

Style and layout (`LocalFrameView::UpdateStyleAndLayout`) was 561 ms of the drag, ResizeObserver callbacks 214 ms. The GPU process spent 277 ms in `CommandBuffer::Flush`. The long frames carry no script or style/layout time in their long-animation-frame entries, which fits time spent waiting on the GPU rather than computing.

### 3.4 Browser panes

With 4 browser panes in the visible tab, 139 `browser_panes_set_rects` batches (4 panes each) were sent over three runs. Request to reply, which the host sends once the panes have moved: **p50 9.3 ms, p90 25.8 ms, max 65 ms.** The IPC round trip alone is 1.2–1.6 ms.

### 3.5 Menus over a browser pane

`browserPanes.screenshot(…, jpeg, q80)`, the capture behind the Linux freeze-frame, eight calls in a row: **34, 58, 88, 169, 174, 175, 183, 190 ms.** A menu that overlaps a pane waits for it before the pane is hidden (`FREEZE_WAIT_CAP_MS = 300`, `platform/pane-overlay.ts:74`). A prewarm on the menu button's pointer-down starts it about 100 ms early.

## 4. Causes, ranked

### 4.1 Terminal refits wait on the GPU (shared with macOS)

Each live refit resizes xterm's WebGL drawing buffer, which Chromium does synchronously with the GPU process. On Linux, as on macOS, the refits' wall time is about three times their CPU time (§3.3). #4658's time budget already caps how many run per frame. What is left is the same trade-off the macOS analysis describes (M1): fewer live refits per frame, or a terminal grid that lags the pane's edge during a drag. Not a Linux-specific problem, and no Linux-specific fix.

### 4.2 The Linux host lays out the whole window once per pane per batch

`apply()` (`browser_panes/bounds.rs:168-185`) receives one batch per window, then on non-Windows calls `resize_browser_pane_view` once per pane. On Linux that function (`browser_pane/creation_views.rs:483-588`) does, for every pane:

- `controller.set_size` and `controller.set_position`;
- `controller.set_visible(…)`, even when visibility didn't change;
- `window.layout()` on the owning window, a full Views layout pass.

So a batch of N panes runs N whole-window layouts and N visibility calls on the UI thread that also handles the window's own resize. With 4 panes that measured 9.3 ms p50 (§3.4). It grows with the pane count: on Windows, even inside the batched task, each pane's move costs about 2 ms (browser-pane analysis §8). Windows fixed this shape with one deferred move per batch (#4420), macOS with a frame-only path for panes already showing (#4657).

### 4.3 Menus over a pane hide it and wait for a screenshot

Linux has no way to clip a native pane, so when a DOM overlay intersects one, the host hides the whole pane (`compute_pane_visible`, `browser_panes/clip.rs:519-536`). The frontend first captures the pane as a JPEG to show in its place (`use-freeze-frame.ts`). That capture is the 34–190 ms of §3.5. macOS stopped hiding panes: it masks the overlapped area of the pane's window with a `CAShapeLayer` hole (`ui_tasks/pane_hole_mask.rs`) and the pane stays live. Windows clips with `SetWindowRgn`.

### 4.4 A pane that only moves waits for the safety poll

On Windows the tile layout calls `notifyPaneReflow()` on every leaf geometry change (`TileLayout.win32.tsx:82`), so every browser pane resamples its placeholder each frame for a moment and pushes the new rect. Linux and macOS don't set `onLeafGeometryChange`. A pane whose placeholder changes size is still caught by its ResizeObserver, but one that only **moves** (a neighbour opened, closed or resized; a split above it) is caught only by the 200 ms `setInterval` in `use-pane-rect-sync.ts`. **Not measured here**; it follows from the code.

### 4.5 Not causes, or not on Linux

- **Host work per resize step:** the Linux host does nothing per resize step, so #4398's trimming has nothing to apply to.
- **Viewport media queries:** none left (#4406).
- **Hidden tabs:** cost nothing during a resize (§3.2).
- **Fill colour:** the Windows analysis measured the uncovered strip as the page's own `#222`; R7 was judged unnecessary there, and nothing here suggests Linux differs.

## 5. Recommendations

| # | Change | Expected effect | Cost / risk |
|---|---|---|---|
| **L1** | **One layout per batch on Linux.** Give `apply()` a Linux branch, like `apply_windows`: for each live pane `set_size` + `set_position`, `set_visible` only when the pane's visibility changes, then **one** `window.layout()` per window. Keep `resize_browser_pane_view` for one-off moves. | The batch's host time drops from N layouts to one; the UI thread is free sooner for the window's own resize. Matters most with many panes. | Small (one function in `bounds.rs`, Linux-gated). Verify on Wayland that panes still land on their rects after a single layout. Measure with §3.4's probe at 4 and 9 panes, before and after. |
| **L2** | **Wire `notifyPaneReflow` on Linux** (and macOS): `onLeafGeometryChange: notifyPaneReflow` in `TileLayout.linux.tsx`. | A pane that only moves follows its placeholder within a frame instead of up to 200 ms. | One line. With L1 in place the extra per-frame batches are cheap. **Measure first** (open and close a pane to the left of a browser pane), since §4.4 is from the code. |
| **L3** | **Clip panes under menus instead of hiding them**, as macOS does. On Linux a pane is a Views overlay in the main window's own compositor, so the clip would be a layer clip or mask, which CEF's public API doesn't expose: a small patch in the `agentmuxai/cef` fork. Windows' Views spike (browser-pane analysis §5.2) needs the same patch, so do it once for both. | Menus over a pane open without the 34–190 ms screenshot wait; the pane stays live under the menu, and the freeze-frame and its 300 ms cap go on Linux. | Medium: a CEF fork patch, a runtime rebuild on three platforms, and the overlay path in `clip.rs`. Do after L1 and L2. |
| **L4** | **Fix the WebGL context-loss reload loop** (macOS M4). Reproduced on Linux with 19 terminals. | No more reload loops for users with many terminals. | Cross-platform; one fix for all three. |
| **L5** | **Measure a real drag on Linux.** GNOME's interactive resize throttles configure events to the client's frame callbacks, which the scripted driver bypasses. `ydotool` can drive a real mouse drag on Wayland (another agent on this host already uses it), so this needs no person. | Shows whether a real drag behaves like the stepped one, and how far panes trail the edge in pixels (with the Mutter screencast capture used for the dock checks). | None to measure. |
| **L6** | **Leave the terminal refit trade-off to the shared work** (macOS M1). | Same as macOS. | Shared, not Linux work. |

**Order:** L1, then L2 (both small, and L2 benefits from L1), then L5 to measure, then L3. L4 is independent.

## 6. Side findings

- **`--disable-features` is passed twice.** The renderer's command line (`env.mjs`) has `disable-features=GlicActorUi,AutofillActorMode,LensOverlay,KillOnInvalidNavigationHeaders` and then the host's own `--disable-features=CalculateNativeWinOcclusion,…`. When a switch repeats, Chromium's `CommandLine` keeps one value, so one of the two lists may not apply. Not a paint problem and not Linux-only; worth a check by whoever owns the switch setup (`app/mod.rs:645`).
- **The measurement scripts are cross-platform** apart from the two Swift samplers: `env.mjs`, `resize-frames.mjs`, `setup-state.mjs`, `trace-cpu.mjs` and `scripts/ui-screenshots/trace-capture.mjs` ran unchanged on Linux. `setup-state.mjs` has 9223 hard-coded, so the dev build needs `AGENTMUX_CDP_PORT=9223`. They live under `scripts/perf/macos-paint/`; a neutral name would help the next person.

## 7. Reproducing

With a dev build on CDP port 9223 (`AGENTMUX_CDP_PORT=9223 task dev`), from `scripts/perf/macos-paint/`:

- `node env.mjs`: GPU backend, features, DPR and idle frame rate.
- `node setup-state.mjs 4 4`: four tabs of four terminals (keep the total at 16 or fewer, §1).
- `node resize-frames.mjs --port 9223 --runs 3 --label <name> --brief`: §3.2.
- `node ../../ui-screenshots/trace-capture.mjs 9223 window_transparent 6 /tmp/t.json` during a resize, then `node trace-cpu.mjs /tmp/t.json`: §3.3. Keep traces outside the repo: Vite reloads the page for any file change under it.
- §3.4: wrap `window.fetch` in the page over CDP and time each request whose body names `browser_panes_set_rects`.
