# macOS window paint: what the Windows work already gives us, and what macOS still needs

**Date:** 2026-10-10
**Status:** analysis. §2 is the status of the Windows paint work, §3 what carries over, §5 the macOS measurements, §6 the causes, §7 the recommendations. Nothing in this PR changes product code; the only code is the measurement scripts in `scripts/perf/macos-paint/`.
**Author:** AgentO
**Trigger:** Repo owner, 2026-10-10: *"we did extensive work on windows optimizing the agentmux windows paint, find that work, whats the status, and I want you to investigate how we can improve macos. research, write report to file."*
**Related:** `ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md` (the Windows resize work), `ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md`, `ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md`, `ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md`, `SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md`.

Code citations are against `main` @ `fd6eae06f`. "Measured" means measured on the machine and build in §5, on a shared machine whose load varied (§5.1): treat frame counts as ±10 and trust ratios and traces more than single runs.

---

## 1. Summary

**Where the Windows work stands.** It is two tracks plus a startup gate, all merged (§2):

- **Window resize** (analysis of 2026-10-06; #4396, #4398, #4399, #4404, #4406, #4408, #4415, #4420): the page went from 30 to 92–97 frames of ~96 in the measured Windows cases. R1–R6 and R8 are shipped; R7 (fill colour) was measured and judged unnecessary. Still open: the host holding the native window's new size until the renderer's frame is ready, the agent pane's remaining ~4.5 ms per phase, and the Windows browser-pane Views spike.
- **Window-tab switching** (#3239, #3686/#3687/#3688, #4107, #4108, #4132, #4140): a warm switch paints in one frame. Recommendations 3–5 of the 09-30 analysis are open.
- **Startup paint gate** (#2968): Windows and Linux only.

**Most of it is cross-platform frontend code that already runs on macOS.** Only three pieces are Windows-only: the host's per-`WM_SIZING` trimming (#4398), the batched `DeferWindowPos` pane apply (#4420), and the paint gate (#2968). The Windows measurements were never repeated on macOS, so nobody knew how much of the gain had carried over.

**What I measured on macOS** (M2 MacBook, macOS 26.5.2, Retina at 60 Hz, CEF 154, ANGLE on Metal with Skia Graphite):

| Case | Result |
|---|---|
| Empty window, hidden tabs holding 16 terminals, a second window, window moves with 4 browser panes, warm tab switches | **Fine.** 86–89 frames for ~84, tab switches 10–34 ms (one 193 ms outlier), panes follow a move with 0 px drift. The Windows skip-hidden-tabs fix works here (§5.3). |
| A visible tab with 4 terminals | **Not fine, by an amount that varies.** 41–75 frames of ~83 over the first sessions (median 51, frame gap 33 ms), 68–71 of ~80 in a later one. The renderer main thread was 96% busy in every trace, and the terminals' live refits are the largest single item (§6.1). |
| A visible tab with 4 browser panes | **The page keeps up (77–92 frames), the panes do not.** The panes on the dragged edge are behind it in **91–93% of samples taken from the drag's start to 300 ms after it** (median 17–20 pt, p90 60–100 pt) and settle 0.26–0.4 s after it; the browser process's UI thread is 68% busy (§5.6, §6.2). |

**What is not the lever** (each tested, §9): host INFO logging, device-pixel ratio, `window:keepinactivetabslaidout`, the DOM terminal renderer, a transparent background on secondary windows, Skia Graphite (turning it off drops the whole app to software rendering).

**Recommendations, in order** (§7): (1) make the terminals' live refits cost-aware, since each refit is a ~6 ms synchronous GPU wait that uses under 2 ms of CPU; (2) rework the macOS browser-pane geometry task: frame-only updates for visible panes, one task per window per batch, no per-tick re-show and no 50 ms reaffirm; (3) measure a real mouse drag, which my driver cannot reproduce, with the recorder in §11 (armed twice for the owner, no drag was captured, so still open). Four smaller items follow.

---

## 2. The Windows paint work: inventory and status

Merged PRs, with where each runs. "Shared" is frontend TypeScript or CSS and runs on every platform.

### 2.1 Window resize (all 2026-10-06 unless noted)

| PR | What | Runs on |
|---|---|---|
| #4396 | R1 + R3: `windowResizing()` signal from the DOM `resize` event (`platform/window-resize.ts`); hidden window tabs get `content-visibility: hidden` during a resize and catch up one per idle slot; the tile layout takes the ResizeObserver's size instead of forcing a layout | Shared |
| #4398 | R4 + R5: `windowresize:tick` only when Shift is held; one trailing writer instead of a thread per position report; WinEvent log demoted | **Windows host** (`wndproc.rs`, `wrr/`) |
| #4399 | R6: terminals refit every frame, only the PTY size is debounced; the System Info 150 ms timer removed | Shared |
| #4404 | R8: `.tile-layout` `animate` class no longer follows resizing; System Info plots use a fixed class and a 100 ms redraw throttle; `PaneTabStrip` overflow read deferred | Shared |
| #4406 | Tailwind's unused `.container` and its five width media queries removed (`@source not inline("container")`) | Shared (CSS) |
| #4408 | `liveRefits`: at most 2 terminals refit per frame, the rest take turns | Shared |
| #4415 (10-07) | Agent list measures resized rows in one pass | Shared |
| #4420 (10-07) | Browser panes: one batched `browser_panes_set_rects` per window per frame; the host merges and applies it | Frontend shared; the apply is `BeginDeferWindowPos` on **Windows only** (`browser_panes/bounds.rs:190`) |

Windows result (from the Windows analysis): 5 tabs and 16 terminals went from 30–34 frames of ~96 to 92–95; an agent-pane tab from 55 to 96–97; a fast drag over a mixed tab from 68 to 79 frames with the page one step behind the edge, which the analysis calls the floor for a page (§12–13 there).

### 2.2 Window-tab switching and startup

| PR | What | Runs on |
|---|---|---|
| #3239 (09-15) | `content-visibility: hidden` for inactive tabs, View Transition | Shared |
| #3686, #3687, #3688 (09-24) | `window:keepinactivetabslaidout` (default on); no reveal gate for a tab already shown | Shared |
| #4107, #4108 (09-30) | Explicit transition lists plus a CI gate; warm switch in one frame; a new tab built in parallel | Shared |
| #4132, #4140 (10-01) | No flash on close or first switch; a new tab's pill selected in its first frame | Shared |
| #2968 (09-04) | The Linux paint gate ported to Windows: the window never shows blank | **Windows and Linux** (`client/navigation.rs`: `#[cfg(any(target_os = "linux", target_os = "windows"))]`); on macOS `window.show()` is unconditional |

### 2.3 What is not done

- **Host holds the native window's new size until the renderer's frame is ready** (resize analysis §12). No PR, spec or issue.
- **Agent-pane busy frame**, ~17.8 ms in four phases of ~4.5 ms (paint, ResizeObserver callbacks, style and layout, other) (§13 there).
- **Windows browser panes as Views overlays** (the §5.2 spike), GPU cost per pane, hidden-tab pane throttling.
- **Tab-switch recommendations 3–5** (focus in the same task, a compositor layer per tab, pre-painting hidden tabs) and the planned `scripts/tab-switch-frames.mjs`, which was never added.
- **`SPEC_WINDOW_RESIZE_NO_PAINT_DELAY` follow-ups F1–F3**; F2 (the macOS 50 ms reaffirm) and F3 (theme background as CEF `background_color`) are untouched.
- **Doc inconsistency:** the resize analysis calls that spec "proposed, never implemented"; the spec's own status says §4.1, §4.2 and §4.4 shipped in #4399, which is right.

---

## 3. What carries over to macOS, and what does not

| Windows change | On macOS |
|---|---|
| Skip hidden tabs while resizing (#4396) | **Works.** §5.3: 16 hidden terminals cost nothing. |
| One-frame warm tab switch (#4107) | **Works.** §5.5. |
| Terminal live refit + turns (#4399, #4408) | **Runs, and is now the main cost** (§6.1). |
| Tailwind breakpoints, `animate` class, plots, strips (#4404, #4406) | Shared, applied; not separately re-measured. |
| Host trimming (#4398) | **Does not exist on macOS, and mostly has no counterpart to trim.** The macOS host has no resize hook, no `windowresize:*` events and no position stream (`wrr::install_hooks` is a no-op stub off Windows, `wrr/mod.rs:35-40`). |
| Batched pane apply (#4420) | **Not applied.** The frontend sends one batch per window per frame, but the macOS host then loops over the panes and runs the full first-run task for each (`browser_panes/bounds.rs:179-181`). |
| Paint gate (#2968) | Not ported. Unmeasured (§10). |

---

## 4. How macOS differs

| | Windows | macOS |
|---|---|---|
| Main window | CEF Views, sized by Chromium's own `HWNDMessageHandler` | CEF Views; frameless `CefNSWindow`; the traffic lights are DOM buttons (`window-controls.darwin.tsx`) |
| Browser panes | `WS_CHILD` wrapper HWND plus a CEF child, moved with `DeferWindowPos` | **Each pane is its own `NativeWidgetMacNSWindow`** (`add_overlay_view`, `browser_pane/creation_views.rs:199-205`), sized by raw Objective-C `setFrame:display:YES` because CEF's `set_size`, `set_position` and `layout()` are no-ops there |
| Live resize | `WM_SIZING` / `WM_EXITSIZEMOVE` handled, `windowresize:*` events | Not observed by the host at all (`windowEdgeResize.ts:393`: inert on macOS and Linux) |
| Window drag | Host manual move loop | Nested `nextEventMatchingMask` loop on the UI thread, `set_bounds` per `LeftMouseDragged` (`ui_tasks/drag.rs:189-310`); floaters are JS-driven over IPC |
| Position persistence | WinEvent hook into a trailing writer | None found (unverified how or whether macOS persists window rects) |
| GPU stack | D3D11 ANGLE, DirectComposition | **Untouched Chromium defaults**: no `use-angle`, `use-gl`, `ignore-gpu-blocklist`, `enable-gpu-rasterization` or frame-rate switch is set for macOS (`app/mod.rs:526-1061`). Measured: ANGLE on Metal, Skia Graphite on. |
| Minimum size | 380 px (`app/mod.rs:80-85`, `cfg(windows)`) | None |
| Background colour | Main window opaque unless transparency is on | Main `0xFF000000`/`0x00000000` by `alpha_capable` (`app/mod.rs:1106`); **secondary windows always `0x00000000`** (`ui_tasks/window.rs:1650`) |
| Compositing under resize | DWM composites the page and the panes separately | Chromium's Mac port **holds the window and its contents in lock-step**: its UI thread waits inside AppKit drawing for a compositor frame the size of the view, up to a timeout (`ui/accelerated_widget_mac/window_resize_helper_mac.h`). Whether that path is active for CEF 154 Views windows driven by `set_bounds` was not verified. |

The last row matters for how to read slowness. The Windows analysis found that a slow renderer shows as a grey band at the window edge, because DWM presents the new window size before the page has a frame. Chromium's Mac design aims to avoid that by waiting, so a slow renderer should show up on macOS as the window's size updates arriving in coarse steps, not as a gap. I could not tell the two apart with a stepped driver (§5.2), so §11 gives a recorder for the real drag.

---

## 5. Measurements on this Mac

### 5.1 Setup and limits

- **Machine:** Apple M2, 8 cores, 24 GB; macOS 26.5.2; built-in Liquid Retina, 2560×1664 (1710×1112 points), **DPR 2, 60 Hz** (idle `requestAnimationFrame` gap 16.7 ms). A 120 Hz ProMotion machine halves the frame budget to 8.3 ms and would be affected more; I could not test one.
- **Build:** `task dev` of `main` @ `fd6eae06f` (v0.59.18), CEF 154.0.8037.58, `AGENTMUX_DISABLE_CLOUD_SUBSCRIBER=1`. `SystemInfo.getInfo`: `ANGLE (Apple, ANGLE Metal Renderer: Apple M2)`, `gpu_compositing`, `rasterization` and `skia_graphite` enabled. Every window reports alpha 1 in `CGWindowList`. macOS Reduce Motion was on.
- **Shared machine:** other agents' dev builds, compiles and the installed app ran throughout. The 1-minute load average was 3.2–7.7 for most runs and spiked to 17 once (that run was discarded). The same scenario varied by ±10–15 frames from this alone.
- **Driver:** the host's own `set_window_rect` command, called from Node (so it adds no work to the page's main thread), 80 steps of 10 points every 16 ms, shrinking 400 points and growing back, like the Windows `SetWindowPos` stepping. **It is not a mouse drag**: it sets bounds directly and does not enter AppKit's live-resize loop. The same command exists on Windows (`SetWindowPos`), so these probes also run there.
- **Probes:** per-frame `requestAnimationFrame` gaps, `resize` events and long-animation-frame entries in the page; Chromium traces of all processes with wall and thread-CPU time; `CGWindowListCopyWindowInfo` sampling of the native windows at ~120–170 Hz (needs no Screen Recording or Accessibility permission, since it reads bounds only). Scripts are in `scripts/perf/macos-paint/` (§11).
- **What I could not measure:** a real mouse drag; how far the page trails the window edge (that needs pixels, which needs Screen Recording permission, which I did not request); a ProMotion display; an Intel Mac; the installed (packaged) build.

### 5.2 Frames during the stepped resize

"Ideal" is driver time ÷ 16.7 ms (~80–90). "Sizes seen" is how many of the 80 sent widths reached the page.

| Case | Runs | Frames (ideal ~83) | Frame gap p50 / p90 | Long frames | Sizes seen |
|---|---|---|---|---|---|
| Empty window, 1 tab | 3 | 81–82 | 16.7 / 17.4–17.6 | 0–1 | 38–41 |
| **Empty visible tab, 16 terminals in hidden tabs** | 3 | **86–89** | 16.7 / 17.6–18 | 0–1 | 38–41 |
| Second window, default tab | 4 | 88–89 | 16.7 / 17.5–17.9 | 0 | 39–41 |
| **Visible tab with 4 terminals** (+12 hidden) | 41 default-state runs | **median 51, range 41–75** | 33 / 34–66 | 2–5 | 26–37 |
| Same, `window:keepinactivetabslaidout` off, alternating | 4 on, 4 off | on 50–56, off 49–58 | same | 2–4 | 30–37 |
| Visible tab with 4 browser panes | 16 | 77–92 | 16.7 / 17.6–33 | 1–4 | 22–30 |

Notes:

- About half the sent widths reach the page in every case, including the empty window (39–41 of 80). The Windows 1-tab row has the same ratio (38 events, 73 frames), so this is Chromium's one-resize-in-flight behaviour, not a macOS problem.
- The 4-terminal rows moved with the machine: the first runs after a page reload were often 57–75; a stable stretch at load ~6–7 gave 41–44 six times running; a later session (fresh instance, load 3.5–5) gave 68–71 of ~80. In the traces the refit's wait varied with it, from ~7.5 ms to ~4.5 ms per refit. The wait is on the GPU process, so other apps' GPU work (the machine was running the installed AgentMux and other dev windows) plausibly moves it; I did not isolate that.

### 5.3 Hidden tabs cost nothing

An empty tab with the 16 terminals in the other four tabs renders 86–89 frames. With `keepinactivetabslaidout` on or off the 4-terminal tab scores the same. This is #4396 working on macOS; it also means the setting is not worth revisiting here.

### 5.4 Where the time goes (trace of a 1.5 s drag, 4 visible terminals)

| Thread | Wall busy | Thread-CPU busy |
|---|---|---|
| Renderer main | **96%** | 77% |
| GPU process main | 40% | 34% |
| Browser UI thread | 17% | 12% |

On the renderer main thread, in 1.5 s: style and layout 610 ms, paint 286 ms, JavaScript 587 ms (a second trace gave 595 / 299 / 538). By function (top-level calls):

| Function | Calls | Wall | Thread CPU |
|---|---|---|---|
| `term-resize-policy.ts` `onFrame` (the terminals' live refits) | 47 | **354 ms** | **81 ms** |
| `sysinfo-plot.tsx` | 44 | 69 ms | 81 ms |
| `useDimensions.tsx` | 44 | 53 ms | 51 ms |
| `use-widget-bar-responsive.ts` | 44 | 48 ms | 44 ms |

The first row is the finding: a refit takes ~7.5 ms of wall time and ~1.7 ms of CPU. It is mostly **waiting**, not computing. The renderer blocks on a synchronous GPU round trip every time xterm's WebGL addon resizes its drawing buffer, which the Windows analysis also saw (§12 there). The GPU process spends about 400 ms of its 1.5 s in `CommandBuffer::Flush`.

Browser-pane tab, same drag, a calmer moment (load 3.7): the page is fine (92 frames), but the **browser process's UI thread is 68% busy on the wall clock (30% CPU)** against 17% in the terminal case, and the GPU main thread is 39% CPU. The UI thread is the one that runs the pane tasks (§6.2) and the window's own resize.

### 5.5 Window-tab switches and window moves

- **Warm tab switch**, 45 switches across five tabs (16 terminals, a tab of browser panes), in two series: `setActiveTab` to the destination being visible, **median 16.7 and 23 ms (one frame), p90 22 and 34 ms**, one 193 ms outlier in the first series. This matches the Windows "21 ms" result.
- **Window move** with four browser panes, 80 steps of 3 points: the page renders every frame (91–98 frames, p90 17.9 ms) and **every pane window stays at exactly the same offset from the main window in every sample** (0 px drift, three runs). The panes are attached to the main window, so a move needs no host work per tick.

### 5.6 Native pane windows during a resize

Four pane windows, sampled at ~120–170 Hz, 80 steps of 4 points (a wider range makes the layout collapse the right-hand panes near 770 px, which shows as 0×0 windows and is the layout working as designed).

**Only the two panes attached to the window's right edge are judged.** Their right edge must keep the same offset from the window's right edge at every width. The inner panes legitimately move with the layout's proportions. (A first version of this measure judged all four panes over the whole sampling period; it understated how often the edge panes are behind and is not used here. Its settle times are still valid, because every pane must return to its resting place, and they agree: 250–420 ms. The window judged here includes 300 ms after the last resize, which can add a few off-position samples; most of the drag is well over that.)

| Run set | Edge panes behind by >2 pt, from the drag's start to 300 ms after it | Median / p90 / p99 / max | Last behind after the window's final size |
|---|---|---|---|
| Default, 3 runs, load 4.6–4.9 | **91–93%** | 17–20 / 60–100 / 96–120 / 100–120 pt | 259–287 ms |

Across the stepped resize the main window passes through only 25–26 of the 80 sent widths with panes open, against 63–76 for the terminals tab in the same session, and each pane window through 7–15 distinct frames. The UI thread that runs the pane tasks is the one that resizes the window.

**The page against the window edge.** Without pixel capture, the nearest measure is the page's own layout width (`innerWidth`, read each frame) against the native window's width at the same moment. With 10-point steps on the terminals tab the page's width differed from the window's by more than 2 pt in 60–63% of samples, p90 10–20 pt: about one step, the floor the Windows analysis describes (§12 there). With 4-point steps on the browser-pane tab it was 29–37%, p90 8–20 pt. This is a scripted drag; a real one was not captured (§11).

---

## 6. Causes, ranked

### 6.1 Terminal live refits stall the renderer on the GPU (the visible-terminals case)

`liveRefits` (`term-resize-policy.ts:60-114`) allows 2 terminals to refit per frame whatever it costs. Each refit resizes xterm's WebGL drawing buffer, which Chromium does synchronously with the GPU process. Measured: ~3–6 ms of waiting per refit (the wall time was ~4.5–7.5 ms in different sessions) and under 2 ms of CPU, so two refits can use most of a 16.7 ms frame doing nothing. The refit also forces a layout (xterm's `fit()` reads the container).

Experiment (temporary edit to the scheduler, since reverted; frames of ~83 ideal):

| Policy | Runs | Median frames | Notes |
|---|---|---|---|
| Today: 2 per frame | pooled 41 | 51 | |
| 1 per frame | 10 | 58 | modest; fewer long frames |
| 1 every other frame | 10 | 69 | frame gap p50 back to 16.7 ms |
| **No live refits (ceiling)** | 10 | **80** | the renderer keeps up; this is the most the terminals can cost |

The interleaved subset on its own, same load window (a fairer comparison, but only 6 runs per arm), gave medians of 60 (today), 64 (1 per frame), 65 (every other frame) and 76 (none). The ordering follows the refit rate in every cut, but individual arms overlap, so **treat the size of the gain from the middle policies as unproven**; the ceiling is what the data supports.

DPR does not appear to matter. With the page's device-pixel ratio emulated to 1 the frames were 51–63 against 57–66 at 2. That fits a fixed per-resize cost, but emulation is imperfect.

### 6.2 Browser-pane geometry: a heavy task per pane per tick, with a re-show

On macOS the frontend's batch reaches `apply()` and the loop calls `resize_browser_pane_view` for each pane (`browser_panes/bounds.rs:179-181`). That function posts `SetPaneBoundsViewsTask` with `retry = 0` every time (`browser_pane/creation_views.rs:494-545`). The `retry == 0` path (`ui_tasks/pane_geometry.rs`):

- calls `controller.set_visible(1)` **unconditionally** (`:172-177`), which per the code's own comment schedules a deferred Views layout that resets the pane's NSWindow frame to 0×0;
- enumerates every window in `[NSApp windows]`, making about five Objective-C calls per window and logging one INFO line per window (`:180-210`);
- runs `setFrame:display:YES` (`:298`), which displays the pane synchronously, plus a `windowNumberAtPoint` diagnostic and more INFO logging (`:311-377`);
- re-keys the main window and re-orders the overlay (`:400-423`);
- and posts **the same task again 50 ms later** with the rect captured now (`:816-829`), to repair the frame that `set_visible(1)` reset.

So every rect update does a re-show, a rescan and a repair, per pane. During a resize the reaffirm can also re-apply a rect that is already stale (`SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md` F2). Measured effects: the browser UI thread 68% busy, the main window receiving fewer sizes than without panes (25–26 against 63–76 in the same session), the panes on the dragged edge behind it in 91–93% of samples from the drag's start to 300 ms after it, and settling 0.26–0.4 s after it. The Windows analysis found the same shape (a request queue, median 542 ms) and fixed it with one batched apply; macOS never got that half.

The host also logged 1,000–1,800 lines per 1.4 s drag in this build, ~450 a second of them one-per-window `ObjC task NSApp window` lines. **That is not the cause**: with `RUST_LOG=warn` (10 lines per run) the pane lag was the same (§5.6). It is worth removing anyway, since dev builds also write each line synchronously to stderr and every build formats it twice.

### 6.3 Small per-frame JavaScript

`sysinfo-plot` (50–70 ms), `useDimensions` (43–53 ms) and the widget bar (41–48 ms) add up to ~150 ms of the 1.5 s drag, about a tenth of the renderer's time. Worth a look once 6.1 is fixed; not before.

### 6.4 Not measured: a real drag

My driver sets bounds directly. A mouse drag goes through AppKit's live-resize loop, where Chromium's Mac port waits for a matching frame (§4), and for a window *move* AgentMux runs its own nested event loop (`ui_tasks/drag.rs`). I measured moves and resizes with direct bounds changes only. §11 gives a way to record the real thing.

---

## 7. Recommendations

| # | Change | Expected effect | Cost / risk |
|---|---|---|---|
| **M1** | **Cost-aware terminal live refits.** Replace the fixed 2 per frame in `createRefitScheduler` with a budget from the measured wall time of recent refits (an EMA per terminal), so a frame never starts a refit it cannot finish, and refits take turns at a rate the frame can afford. Keep the settle refit and the PTY debounce. | Toward the ceiling: median ~51 → up to ~80 frames of ~83 in the 4-terminal case. Middle policies measured +7 to +18 (unproven, §6.1). | Small (one file plus a test). **Product decision:** how stale a terminal's grid may look mid-drag. The Windows work chose "every frame" to keep text on the edge; this trades that for a steady frame rate. Pane backgrounds still follow the edge. |
| **M2** | **Rework the macOS pane geometry task.** (a) A resize of an already-visible pane takes the frame-only path: no `set_visible(1)`, no 50 ms reaffirm. (b) One UI task per window per batch, mirroring `apply_windows`. (c) Remember the overlay `NSWindow` instead of scanning `[NSApp windows]` each time. (d) Drop the `windowNumberAtPoint` diagnostic and the per-window INFO logs. (e) Skip the key-window, `orderFront:` and `set_focus` steps on repeated resize ticks. | The browser UI thread frees up (68% busy today); panes land within a frame or two instead of ~0.3 s; the main window should step more finely. | Medium (Rust on the UI thread). The reaffirm exists because CEF's Views layout resets the frame after a re-show, so the hypothesis is that skipping the re-show removes the need; **prototype and verify with `drag.mjs --mode scripted` before relying on it.** The Windows `apply_windows` is the template. |
| **M3** | **Measure a real drag** with the passive recorder (§11), on this Mac and on a ProMotion machine, before and after M1/M2. It settles whether the lock-step wait (§4) changes the picture and how far the page trails the edge. | Replaces the unmeasured parts of §6.4 with data. | None. 60 seconds of a person dragging. |
| **M4** | **Don't reload the page on WebGL context loss, and release contexts held by hidden terminals.** Past 16 live terminals Chromium drops WebGL contexts, xterm falls back to its DOM renderer, and the frontend's recovery reloads the page up to three times (§10). Hidden-tab terminals each hold a context while painting nothing. | Removes a reload loop for users with many terminals; frees GPU contexts (not measured how much that helps the 34% GPU CPU). | Small to medium; touches terminal lifecycle. Affects every platform. |
| **M5** | Add the missing macOS **minimum window size** (`minimum_size` is `cfg(windows)`, `app/mod.rs:81`). | Stops the layout collapsing panes at very small widths (seen at ~770 px in §5.6). | Small. |
| **M6** | **Measure a first-paint gap on macOS** before porting #2968. | Unknown; the Windows gate fixed a blank window on startup. | None to measure. |
| **M7** | **Doc and comment hygiene:** `TileLayout.darwin.tsx` still says "WKWebView"; `SPEC_MACOS_RESIZE_HANDLES_V2.md` is Tauri-era; the host runs CEF 154, not 152; the resize analysis mislabels the no-paint-delay spec. | Fewer wrong turns for the next person. | Trivial. |

**Order:** M3 first if a person can spare a minute (it decides how much M1 and M2 matter), otherwise M2 then M1. M2 is the one with a clear structural cause and a precedent on Windows.

---

## 8. DRY and modularization opportunities

- **One pane-geometry apply path.** `apply()` has a Windows branch (`apply_windows`, one deferred move per parent) and a loop for everything else that calls the per-pane function. The batch and its dedupe are shared; the platform step should be one function per platform behind the same signature, so macOS and Linux get the same "once per window per frame" shape as Windows by construction.
- **One "window is resizing" signal.** `windowResizing()` (from the DOM `resize` event) is cross-platform and drives the tab skip. The host's `windowresize:begin|tick|end` exist on Windows only and serve just the Shift-edge resize. Either derive that feature from the DOM signal or provide the events on all platforms, so the two do not drift.
- **One resize scheduler.** The Windows analysis (§6) proposed `platform/resize-scheduler.ts` with an every-frame lane and a settle lane. It is still unbuilt, and `liveRefits`, the System Info 100 ms throttle and the agent list's debounce each decide their own policy. M1's frame-budget logic belongs there.
- **One measurement driver.** `set_window_rect` is the host's own command on every platform. The probes in `scripts/perf/macos-paint/` use it, so the Windows `resizesteps.ps1` can be retired and Windows numbers re-taken with the same scripts. Only the two Swift samplers are macOS-specific.

---

## 9. Tested and ruled out

| Idea | Result |
|---|---|
| Host INFO logging is the pane lag | No. With `RUST_LOG=warn` the pane deviation and off-position share were the same as with INFO logging (measured with the first, all-panes version of the measure, which is fine for comparing the two arms), and the settle time stayed at 250–361 ms against 359–419 ms. Still worth removing. |
| Retina cost (DPR) | No difference at emulated DPR 1 (51–63 vs 57–66 frames). Emulation is imperfect. |
| `window:keepinactivetabslaidout` | No difference on or off (on 50–56, off 49–58). #4396 already covers it. |
| DOM terminal renderer instead of WebGL | Worse in both rounds: 30–59 frames vs 49–67 for WebGL (`term:disablewebgl`). |
| Turn Skia Graphite off | **Not viable.** With `SkiaGraphite` disabled Chromium picked `--use-angle=swiftshader-webgl` and the app ran with `gpu_compositing: disabled_software`. The hardware path on this build depends on Graphite. (Web reports mention Graphite glitches on macOS 26; I saw none, but the option to disable it is not there.) |
| Transparent background on secondary windows costs frames | No. A second window with the transparent `BrowserSettings` colour rendered 88–89 frames of ~83 with a 18.6 ms p99. |
| Panes lag the main window during a move | No. 0 px drift in every sample (§5.5). |

---

## 10. Side findings and open questions

**Side findings**

- **WebGL context loss reloads the page.** Creating 17–19 terminals made Chromium drop contexts; xterm switched those terminals to its DOM renderer (`termwrap.ts:840`), and the frontend's separate `[recovery] WebGL context lost — reloading page` logic reloaded the page three times before suppressing itself. The 16-terminal case in the Windows analysis sits exactly at the limit. Cross-platform; see M4.
- **Vite reloads the page for any file change under the repo root**, including files outside the app (it reloaded the dev window when I added probe scripts inside the repo). Keep dev-time probes outside the repo while a dev window is measuring.
- **The first run after a page reload scored better than later runs** in several series. A plausible cause is terminal scrollback growing; I did not test it.

**Open questions**

1. Is the main window's `NSWindow` opaque? `CGWindowList` alpha says 1, which is the window alpha, not whether the content composites with blending. The host never calls `setOpaque`; reading `isOpaque` needs a one-line debug hook in the host.
2. Is Chromium's Mac resize lock-step active for CEF 154 Views windows driven by `set_bounds`, and does a real mouse drag behave differently (M3)?
3. How far does the page trail the window edge in pixels? The layout-width proxy in §5.6 says about one step in a scripted drag; what is on screen during a real drag needs pixel capture, which needs Screen Recording permission.
4. Does a ProMotion (120 Hz) display change the picture? The frame budget halves.
5. Does macOS have a blank-window gap at first paint that the Windows gate fixed there (M6)?
6. Do pane windows reorder correctly when the main window is hidden behind another app mid-resize? Not tested.

---

## 11. Reproducing, and recording a real drag

The scripts are in `scripts/perf/macos-paint/` and need a dev build (CDP on port 9223) and Node. They use `scripts/ui-screenshots/lib/cdp-client.mjs`. Compile the samplers once (`swiftc -O panesampler.swift -o panesampler`, the same for `winlist.swift`) and run from that directory.

| Script | What it does |
|---|---|
| `env.mjs` | GPU backend, Graphite, idle frame rate, DPR |
| `resize-frames.mjs` | The stepped-resize frame probe of §5.2; `--runs N`, `--brief`, `--step-px` |
| `drag.mjs <cef pid> --mode scripted` | §5.6: the stepped resize with page frames, the page's width against the window's, and the edge panes against the window's edge (`drag-analysis.mjs`, `panesampler.swift`) |
| `drag.mjs <cef pid> --mode armed` | The same for a **real mouse drag**: it waits for the first resize, records until 1.5 s after the last, and reports |
| `move-lag.mjs <cef pid>` | The window-move test of §5.5 |
| `tab-switch.mjs` | Warm tab switches (§5.5) |
| `trace-cpu.mjs`, `trace-threads.mjs` | Wall and CPU attribution of a trace from `scripts/ui-screenshots/trace-capture.mjs` |
| `setup-state.mjs`, `setcfg.mjs`, `gototab.mjs` | Build the tab and terminal state, flip a setting, select a tab |

Keep the total at 16 terminals or fewer (M4).

**Recording a real drag (M3).** Select the tab to test (`node gototab.mjs <index>`), run `node drag.mjs <cef pid> --mode armed`, and drag the window's right edge out and back for a few seconds. Do it on the terminals tab and on the browser-pane tab, then compare with the `--mode scripted` numbers in §5.6. On 2026-10-10 it was armed twice for 15 minutes each and caught no drag, so this is still open.

---

## Appendix A: Sources

- Chromium `ui/accelerated_widget_mac/window_resize_helper_mac.h`: [source](https://chromium.googlesource.com/chromium/src/+/main/ui/accelerated_widget_mac/window_resize_helper_mac.h) and the change that introduced it, [codereview 1352743002](https://codereview.chromium.org/1352743002).
- Electron, [tech talk: window resize behavior](https://electronjs.org/blog/tech-talk-window-resize-behavior): the resize artifacts there were Windows-specific (Present/Commit desynchronisation) and "not visible on macOS".
- Chrome, [Introducing Skia Graphite](https://blog.google/chromium/introducing-skia-graphite-chromes/) and the Chromium 130+ rollout.
